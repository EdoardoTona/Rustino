//! Encryption of the app's secrets (tokens, passwords) with a key that only the current user can
//! use: DPAPI on Windows; on macOS and Linux AES-256-GCM with a random key of the app, kept in the
//! Keychain or in the Secret Service through libsecret (GNOME Keyring, KWallet).
//!
//! The encrypted bytes start with a format version; they decrypt only for the same user and app
//! id on the same platform (on Windows, the same machine too).

use std::ffi::c_char;
use std::panic::catch_unwind;

// Values of the host API (RustinoSafeStorage)
const OK: i32 = 0;
const INVALID_ARGUMENT: i32 = -1;
/// No key store: no Secret Service on Linux, a keychain that can't be opened on macOS
const UNAVAILABLE: i32 = -2;
/// The user refused access to the key or didn't unlock the keychain
const DENIED: i32 = -3;
/// Not bytes of this app and user, or altered
const INVALID_DATA: i32 = -4;

const VERSION: u8 = 1;

#[cfg_attr(target_os = "windows", allow(dead_code))]
#[derive(Debug, PartialEq)]
enum Error {
    Unavailable,
    Denied,
    InvalidData,
}

impl Error {
    fn code(&self) -> i32 {
        match self {
            Error::Unavailable => UNAVAILABLE,
            Error::Denied => DENIED,
            Error::InvalidData => INVALID_DATA,
        }
    }
}

/// 1 when `app_id` can encrypt, 0 when not, -1 for an invalid id. On macOS and Linux it reads
/// the key, creating it the first time: the system may ask the user to unlock the keychain.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_safe_storage_available(app_id: *const c_char) -> i32 {
    catch_unwind(|| match unsafe { app_id_arg(app_id) } {
        Some(app_id) => i32::from(platform::available(&app_id)),
        None => INVALID_ARGUMENT,
    })
    .unwrap_or(0)
}

/// Encrypts the `len` bytes at `data` into a buffer of `*out_len` bytes at `*out`, released with
/// `rustino_safe_storage_free`. Returns 0, or a negative error with nothing in `*out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_safe_storage_encrypt(
    app_id: *const c_char,
    data: *const u8,
    len: usize,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    unsafe { transform(app_id, data, len, out, out_len, platform::encrypt) }
}

/// Decrypts bytes of `rustino_safe_storage_encrypt`, like it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_safe_storage_decrypt(
    app_id: *const c_char,
    data: *const u8,
    len: usize,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    unsafe { transform(app_id, data, len, out, out_len, platform::decrypt) }
}

/// Deletes the key of `app_id`: what it encrypted no longer decrypts, and the next encryption
/// makes a new key. Nothing to delete on Windows, where DPAPI keeps the keys of the user.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_safe_storage_reset(app_id: *const c_char) -> i32 {
    catch_unwind(|| match unsafe { app_id_arg(app_id) } {
        Some(app_id) => match platform::reset(&app_id) {
            Ok(()) => OK,
            Err(error) => error.code(),
        },
        None => INVALID_ARGUMENT,
    })
    .unwrap_or(UNAVAILABLE)
}

/// Releases a buffer of `rustino_safe_storage_encrypt` or `rustino_safe_storage_decrypt`,
/// clearing it first: it may hold the secret.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_safe_storage_free(data: *mut u8, len: usize) {
    let _ = catch_unwind(|| {
        if !data.is_null() {
            let mut buffer = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(data, len)) };
            clear(&mut buffer);
        }
    });
}

unsafe fn transform(
    app_id: *const c_char,
    data: *const u8,
    len: usize,
    out: *mut *mut u8,
    out_len: *mut usize,
    f: fn(&str, &[u8]) -> Result<Vec<u8>, Error>,
) -> i32 {
    catch_unwind(|| {
        if out.is_null() || out_len.is_null() || (data.is_null() && len > 0) {
            return INVALID_ARGUMENT;
        }
        unsafe {
            *out = std::ptr::null_mut();
            *out_len = 0;
        }
        let Some(app_id) = (unsafe { app_id_arg(app_id) }) else {
            return INVALID_ARGUMENT;
        };
        let data = if len == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(data, len) } };
        match f(&app_id, data) {
            Ok(result) => {
                let result = result.into_boxed_slice();
                unsafe {
                    *out_len = result.len();
                    *out = Box::into_raw(result).cast();
                }
                OK
            }
            Err(error) => error.code(),
        }
    })
    .unwrap_or(UNAVAILABLE)
}

unsafe fn app_id_arg(app_id: *const c_char) -> Option<String> {
    unsafe { crate::util::cstr_to_string(app_id) }.filter(|id| crate::autostart::is_valid_app_id(id))
}

/// Overwrites a secret before its memory is released.
fn clear(buffer: &mut [u8]) {
    for byte in buffer.iter_mut() {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
}

/// AES-256-GCM with a key of the app in the store of the platform: `VERSION`, a random 12 byte
/// nonce, the ciphertext and its 16 byte tag.
#[cfg(not(target_os = "windows"))]
mod aead {
    use super::{Error, VERSION, clear, platform::store};
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::sync::{Mutex, MutexGuard};

    const KEY_LEN: usize = 32;
    const NONCE_LEN: usize = 12;

    /// The last key read from the store per app, to decrypt without reading it again
    static KEYS: Mutex<Option<HashMap<String, [u8; KEY_LEN]>>> = Mutex::new(None);

    type Keys = HashMap<String, [u8; KEY_LEN]>;

    fn cache() -> MutexGuard<'static, Option<Keys>> {
        KEYS.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn available(app_id: &str) -> bool {
        let mut keys = cache();
        let Ok(_lock) = lock(app_id) else {
            return false;
        };
        stored_key(keys.get_or_insert_with(HashMap::new), app_id, true).is_ok()
    }

    pub fn encrypt(app_id: &str, data: &[u8]) -> Result<Vec<u8>, Error> {
        let mut keys = cache();
        // Until the bytes are made: a reset in another process waits for them, and after the
        // reset they no longer decrypt, as expected
        let _lock = lock(app_id)?;
        // Never the cached key: another process may have reset it, and bytes encrypted with it
        // would not decrypt after a restart
        let key = stored_key(keys.get_or_insert_with(HashMap::new), app_id, true)?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| Error::Unavailable)?;
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| Error::Unavailable)?;
        let sealed = cipher.encrypt(Nonce::from_slice(&nonce), data).map_err(|_| Error::Unavailable)?;
        let mut out = Vec::with_capacity(1 + NONCE_LEN + sealed.len());
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    pub fn decrypt(app_id: &str, data: &[u8]) -> Result<Vec<u8>, Error> {
        let Some((&VERSION, rest)) = data.split_first() else {
            return Err(Error::InvalidData);
        };
        if rest.len() < NONCE_LEN + 16 {
            return Err(Error::InvalidData);
        }
        let (nonce, sealed) = rest.split_at(NONCE_LEN);
        let open = |key: &[u8; KEY_LEN]| {
            Aes256Gcm::new_from_slice(key)
                .ok()
                .and_then(|cipher| cipher.decrypt(Nonce::from_slice(nonce), sealed).ok())
        };
        let mut keys = cache();
        let keys = keys.get_or_insert_with(HashMap::new);
        let cached = keys.get(app_id).copied();
        if let Some(plain) = cached.as_ref().and_then(open) {
            return Ok(plain);
        }
        // Not read yet, or the bytes are of a newer key (made after a reset in another process).
        // Without a key nothing was encrypted with it: don't make one
        let key = stored_key(keys, app_id, false)?;
        if cached == Some(key) {
            return Err(Error::InvalidData);
        }
        open(&key).ok_or(Error::InvalidData)
    }

    pub fn reset(app_id: &str) -> Result<(), Error> {
        let mut keys = cache();
        let _lock = lock(app_id)?;
        store::delete(app_id)?;
        if let Some(mut key) = keys.get_or_insert_with(HashMap::new).remove(app_id) {
            clear(&mut key);
        }
        Ok(())
    }

    /// The key of the app in the store, which also goes in the cache; `create` makes it when the
    /// store has none, and then the caller holds `lock`.
    fn stored_key(keys: &mut Keys, app_id: &str, create: bool) -> Result<[u8; KEY_LEN], Error> {
        let stored = match store::load(app_id)? {
            Some(stored) => stored,
            None if !create => {
                if let Some(mut key) = keys.remove(app_id) {
                    clear(&mut key);
                }
                return Err(Error::InvalidData);
            }
            None => {
                let mut key = [0u8; KEY_LEN];
                getrandom::fill(&mut key).map_err(|_| Error::Unavailable)?;
                let mut encoded = to_hex(&key);
                clear(&mut key);
                let added = store::add(app_id, encoded.as_bytes());
                clear(unsafe { encoded.as_bytes_mut() });
                added?;
                store::load(app_id)?.ok_or(Error::Unavailable)?
            }
        };
        let key = from_hex(&stored);
        let mut stored = stored;
        clear(&mut stored);
        // A value that isn't a key is left in the store: reset replaces it
        let key = key.ok_or(Error::Unavailable)?;
        keys.insert(app_id.to_owned(), key);
        Ok(key)
    }

    /// Serializes the creation of the key, encryption and reset among the processes of the
    /// user: libsecret replaces an item with the same attributes, so two processes creating the
    /// key at once would each keep their own. Released when the file is dropped.
    fn lock(app_id: &str) -> Result<File, Error> {
        let dir = store::lock_dir().ok_or(Error::Unavailable)?;
        std::fs::create_dir_all(&dir).map_err(|_| Error::Unavailable)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("rustino-safe-storage-{app_id}.lock")))
            .map_err(|_| Error::Unavailable)?;
        file.lock().map_err(|_| Error::Unavailable)?;
        Ok(file)
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn from_hex(text: &[u8]) -> Option<[u8; KEY_LEN]> {
        if text.len() != KEY_LEN * 2 {
            return None;
        }
        let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        let mut key = [0u8; KEY_LEN];
        for (i, pair) in text.chunks(2).enumerate() {
            key[i] = digit(pair[0])? << 4 | digit(pair[1])?;
        }
        Some(key)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hex_round_trips() {
            let key: [u8; KEY_LEN] = std::array::from_fn(|i| (i * 37) as u8);
            assert_eq!(from_hex(to_hex(&key).as_bytes()), Some(key));
        }

        #[test]
        fn hex_of_another_length_or_alphabet_is_rejected() {
            assert_eq!(from_hex(b"00ff"), None);
            assert_eq!(from_hex("zz".repeat(KEY_LEN).as_bytes()), None);
        }

        #[test]
        fn short_or_unversioned_data_is_rejected_before_reading_the_key() {
            assert_eq!(decrypt("com.example.App", &[]), Err(Error::InvalidData));
            assert_eq!(decrypt("com.example.App", &[VERSION; 28]), Err(Error::InvalidData));
            assert_eq!(decrypt("com.example.App", &[VERSION + 1; 64]), Err(Error::InvalidData));
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    pub use super::aead::{available, decrypt, encrypt, reset};

    /// A generic password of the login keychain: service "<app id> Safe Storage", account the
    /// app id. Keychain Access shows it under that name.
    pub mod store {
        use super::super::Error;
        use core_foundation::data::CFData;
        use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass};
        use security_framework::passwords::{delete_generic_password, get_generic_password};
        use std::path::PathBuf;

        const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
        const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
        const ERR_SEC_USER_CANCELED: i32 = -128;
        const ERR_SEC_AUTH_FAILED: i32 = -25293;

        fn service(app_id: &str) -> String {
            format!("{app_id} Safe Storage")
        }

        fn error(code: i32) -> Error {
            match code {
                ERR_SEC_USER_CANCELED | ERR_SEC_AUTH_FAILED => Error::Denied,
                _ => Error::Unavailable,
            }
        }

        pub fn load(app_id: &str) -> Result<Option<Vec<u8>>, Error> {
            match get_generic_password(&service(app_id), app_id) {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(error(e.code())),
            }
        }

        /// Adds the item unless there is one: never replaces a key in use.
        pub fn add(app_id: &str, value: &[u8]) -> Result<(), Error> {
            let mut options = ItemAddOptions::new(ItemAddValue::Data {
                class: ItemClass::generic_password(),
                data: CFData::from_buffer(value),
            });
            let service = service(app_id);
            options.set_service(&service).set_account_name(app_id).set_label(&service);
            match options.add() {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERR_SEC_DUPLICATE_ITEM => Ok(()),
                Err(e) => Err(error(e.code())),
            }
        }

        /// `$TMPDIR` is of the user (and of the app's container in the sandbox)
        pub fn lock_dir() -> Option<PathBuf> {
            Some(std::env::temp_dir())
        }

        pub fn delete(app_id: &str) -> Result<(), Error> {
            match delete_generic_password(&service(app_id), app_id) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
                Err(e) => Err(error(e.code())),
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    pub use super::aead::{available, decrypt, encrypt, reset};

    /// An item of the default collection of the Secret Service, found by the `application`
    /// attribute. libsecret (0.19 or later) is loaded when first needed: without it there's no
    /// safe storage, but the library still loads.
    pub mod store {
        use super::super::Error;
        use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
        use std::path::PathBuf;
        use std::sync::OnceLock;

        /// A `SecretSchema` of one string attribute. Its `int` fields (flags, attribute types,
        /// reserved) are 0 and each one is padded to the next pointer: so the struct is 74
        /// pointer-sized slots, with the name in the first and the attribute name in the third.
        type SecretSchema = [*const c_char; 74];
        /// Only freed, never read
        type GError = c_void;

        type StoreFn = unsafe extern "C" fn(
            *const SecretSchema,
            *const c_char,
            *const c_char,
            *const c_char,
            *mut c_void,
            *mut *mut GError,
            ...
        ) -> c_int;
        type LookupFn =
            unsafe extern "C" fn(*const SecretSchema, *mut c_void, *mut *mut GError, ...) -> *mut c_char;
        type ClearFn = unsafe extern "C" fn(*const SecretSchema, *mut c_void, *mut *mut GError, ...) -> c_int;
        type SearchFn =
            unsafe extern "C" fn(*const SecretSchema, c_int, *mut c_void, *mut *mut GError, ...) -> *mut c_void;

        struct Libsecret {
            _library: libloading::Library,
            store: StoreFn,
            lookup: LookupFn,
            clear: ClearFn,
            /// libsecret 0.19+: tells a locked item from a missing one, which lookup can't
            search: SearchFn,
            password_free: unsafe extern "C" fn(*mut c_char),
            error_free: unsafe extern "C" fn(*mut GError),
            list_length: unsafe extern "C" fn(*mut c_void) -> c_uint,
            list_free_full: unsafe extern "C" fn(*mut c_void, unsafe extern "C" fn(*mut c_void)),
            object_unref: unsafe extern "C" fn(*mut c_void),
        }

        // Function pointers of a library that stays loaded
        unsafe impl Send for Libsecret {}
        unsafe impl Sync for Libsecret {}

        const SCHEMA_NAME: &CStr = c"rustino.SafeStorage";
        const ATTRIBUTE: &CStr = c"application";
        const COLLECTION_DEFAULT: &CStr = c"default";
        const SEARCH_ALL: c_int = 1 << 1;

        fn libsecret() -> Option<&'static Libsecret> {
            static LIBSECRET: OnceLock<Option<Libsecret>> = OnceLock::new();
            LIBSECRET
                .get_or_init(|| unsafe {
                    let library = libloading::Library::new("libsecret-1.so.0").ok()?;
                    // g_* come from GLib and GObject, which libsecret links
                    Some(Libsecret {
                        store: *library.get(b"secret_password_store_sync\0").ok()?,
                        lookup: *library.get(b"secret_password_lookup_sync\0").ok()?,
                        clear: *library.get(b"secret_password_clear_sync\0").ok()?,
                        search: *library.get(b"secret_password_search_sync\0").ok()?,
                        password_free: *library.get(b"secret_password_free\0").ok()?,
                        error_free: *library.get(b"g_error_free\0").ok()?,
                        list_length: *library.get(b"g_list_length\0").ok()?,
                        list_free_full: *library.get(b"g_list_free_full\0").ok()?,
                        object_unref: *library.get(b"g_object_unref\0").ok()?,
                        _library: library,
                    })
                })
                .as_ref()
        }

        fn schema() -> SecretSchema {
            let mut schema = [std::ptr::null(); 74];
            schema[0] = SCHEMA_NAME.as_ptr();
            // Attribute 0: name, then SECRET_SCHEMA_ATTRIBUTE_STRING (0)
            schema[2] = ATTRIBUTE.as_ptr();
            schema
        }

        /// Ok when libsecret reported no error.
        unsafe fn check(lib: &Libsecret, error: *mut GError) -> Result<(), Error> {
            if error.is_null() {
                return Ok(());
            }
            unsafe { (lib.error_free)(error) };
            Err(Error::Unavailable)
        }

        pub fn load(app_id: &str) -> Result<Option<Vec<u8>>, Error> {
            let lib = libsecret().ok_or(Error::Unavailable)?;
            let app_id = CString::new(app_id).map_err(|_| Error::Unavailable)?;
            let schema = schema();
            let mut error = std::ptr::null_mut();
            let null: *const c_char = std::ptr::null();
            let value = unsafe {
                (lib.lookup)(&schema, std::ptr::null_mut(), &mut error, ATTRIBUTE.as_ptr(), app_id.as_ptr(), null)
            };
            unsafe { check(lib, error)? };
            if !value.is_null() {
                let bytes = unsafe { CStr::from_ptr(value) }.to_bytes().to_vec();
                // Clears the secret before freeing it
                unsafe { (lib.password_free)(value) };
                return Ok(Some(bytes));
            }
            // Also nothing when the user didn't unlock the collection: then there is an item
            if count(lib, &app_id)? > 0 {
                return Err(Error::Denied);
            }
            Ok(None)
        }

        /// The items of the app, locked or not.
        fn count(lib: &Libsecret, app_id: &CStr) -> Result<c_uint, Error> {
            let schema = schema();
            let mut error = std::ptr::null_mut();
            let null: *const c_char = std::ptr::null();
            let items = unsafe {
                (lib.search)(&schema, SEARCH_ALL, std::ptr::null_mut(), &mut error, ATTRIBUTE.as_ptr(), app_id.as_ptr(), null)
            };
            unsafe { check(lib, error)? };
            if items.is_null() {
                return Ok(0);
            }
            let count = unsafe { (lib.list_length)(items) };
            unsafe { (lib.list_free_full)(items, lib.object_unref) };
            Ok(count)
        }

        /// libsecret replaces an item with the same attributes: the caller holds the lock of the
        /// app, and `load` found none just before.
        pub fn add(app_id: &str, value: &[u8]) -> Result<(), Error> {
            let lib = libsecret().ok_or(Error::Unavailable)?;
            let label = CString::new(format!("{app_id} Safe Storage")).map_err(|_| Error::Unavailable)?;
            let app_id = CString::new(app_id).map_err(|_| Error::Unavailable)?;
            let value = CString::new(value).map_err(|_| Error::Unavailable)?;
            let schema = schema();
            let mut error = std::ptr::null_mut();
            let null: *const c_char = std::ptr::null();
            let stored = unsafe {
                (lib.store)(
                    &schema,
                    COLLECTION_DEFAULT.as_ptr(),
                    label.as_ptr(),
                    value.as_ptr(),
                    std::ptr::null_mut(),
                    &mut error,
                    ATTRIBUTE.as_ptr(),
                    app_id.as_ptr(),
                    null,
                )
            };
            let mut value = value.into_bytes();
            super::super::clear(&mut value);
            unsafe { check(lib, error)? };
            if stored == 0 { Err(Error::Unavailable) } else { Ok(()) }
        }

        /// The runtime directory of the user (a tmpfs of the session), else its cache directory.
        pub fn lock_dir() -> Option<PathBuf> {
            let absolute = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|dir| dir.is_absolute());
            absolute("XDG_RUNTIME_DIR")
                .or_else(|| absolute("XDG_CACHE_HOME"))
                .or_else(|| absolute("HOME").map(|home| home.join(".cache")))
        }

        pub fn delete(app_id: &str) -> Result<(), Error> {
            // Unlocks the item, or tells that the user refused: clear only deletes unlocked items
            match load(app_id)? {
                None => return Ok(()),
                Some(mut value) => super::super::clear(&mut value),
            }
            let lib = libsecret().ok_or(Error::Unavailable)?;
            let id = CString::new(app_id).map_err(|_| Error::Unavailable)?;
            let schema = schema();
            let mut error = std::ptr::null_mut();
            let null: *const c_char = std::ptr::null();
            unsafe {
                (lib.clear)(&schema, std::ptr::null_mut(), &mut error, ATTRIBUTE.as_ptr(), id.as_ptr(), null);
                check(lib, error)?;
            }
            // FALSE without an error both when nothing was deleted and when the item stayed
            // locked: it must be gone, or the old bytes would decrypt again after an unlock
            if count(lib, &id)? > 0 { Err(Error::Denied) } else { Ok(()) }
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{Error, VERSION};
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows::core::PCWSTR;

    pub fn available(_app_id: &str) -> bool {
        true
    }

    /// `VERSION` and the DPAPI blob of the user, with the app id as entropy: another app of the
    /// same user can't decrypt it by calling DPAPI alone.
    pub fn encrypt(app_id: &str, data: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = vec![VERSION];
        out.extend_from_slice(&protect(app_id, data, true)?);
        Ok(out)
    }

    pub fn decrypt(app_id: &str, data: &[u8]) -> Result<Vec<u8>, Error> {
        match data.split_first() {
            Some((&VERSION, blob)) if !blob.is_empty() => protect(app_id, blob, false),
            _ => Err(Error::InvalidData),
        }
    }

    pub fn reset(_app_id: &str) -> Result<(), Error> {
        Ok(())
    }

    fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, Error> {
        Ok(CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(bytes.len()).map_err(|_| Error::InvalidData)?,
            pbData: bytes.as_ptr().cast_mut(),
        })
    }

    fn protect(app_id: &str, data: &[u8], encrypt: bool) -> Result<Vec<u8>, Error> {
        let input = blob(data)?;
        let entropy = blob(app_id.as_bytes())?;
        let mut output = CRYPT_INTEGER_BLOB::default();
        let result = unsafe {
            if encrypt {
                CryptProtectData(&input, PCWSTR::null(), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            } else {
                CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            }
        };
        if result.is_err() {
            return Err(if encrypt { Error::Unavailable } else { Error::InvalidData });
        }
        let bytes = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
        let out = bytes.to_vec();
        super::clear(bytes);
        unsafe { LocalFree(Some(HLOCAL(output.pbData.cast()))) };
        Ok(out)
    }
}
