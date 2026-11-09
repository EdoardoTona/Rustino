//! Downloads, saved where the host or the user chooses.
//!
//! The webview writes every download to a temporary file, moved to its destination when complete.
//! The save dialog opens after the webview's handler returns: wry keeps the handler borrowed while
//! it runs, so a second download starting while a modal dialog runs inside it would panic.

use std::ffi::{CString, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::commands::RustinoCommand;
use crate::dialogs::{self, DialogParams, FileDialogKind};
use crate::invoke::Task;
use crate::window::CommandProxy;

use super::{DownloadResponse, Host};

enum Destination {
    /// The save dialog is open
    Asking,
    Chosen(PathBuf),
    Canceled,
}

pub(super) struct Download {
    id: u64,
    url: String,
    /// The file the webview writes, alone in its folder
    temp: PathBuf,
    /// Where the webview would have saved the file, in the Downloads folder
    suggested: PathBuf,
    destination: Destination,
    /// Whether the webview wrote the whole file, once it's done
    finished: Option<bool>,
}

#[derive(Default)]
pub(super) struct Downloads {
    next_id: u64,
    list: Vec<Download>,
}

impl Downloads {
    fn add(&mut self, url: String, temp: PathBuf, suggested: PathBuf, destination: Destination) -> u64 {
        self.next_id += 1;
        self.list.push(Download {
            id: self.next_id,
            url,
            temp,
            suggested,
            destination,
            finished: None,
        });
        self.next_id
    }

    /// The webview is done with a download: returns it once its destination is known.
    pub(super) fn finish(&mut self, url: &str, path: Option<&Path>, success: bool) -> Option<Download> {
        // WKWebView reports only the URL
        let index = path
            .and_then(|path| self.list.iter().position(|d| d.temp == path))
            .or_else(|| self.list.iter().position(|d| d.finished.is_none() && d.url == url))?;
        self.list[index].finished = Some(success);
        self.take_if_done(index)
    }

    /// The user chose the destination: returns the download once the webview is done with it.
    fn choose(&mut self, id: u64, destination: Option<PathBuf>) -> Option<Download> {
        let index = self.list.iter().position(|d| d.id == id)?;
        self.list[index].destination = destination.map_or(Destination::Canceled, Destination::Chosen);
        self.take_if_done(index)
    }

    fn take_if_done(&mut self, index: usize) -> Option<Download> {
        let download = &self.list[index];
        let done = download.finished.is_some() && !matches!(download.destination, Destination::Asking);
        done.then(|| self.list.remove(index))
    }
}

impl Drop for Downloads {
    /// The window closed: the webview no longer writes the files
    fn drop(&mut self) {
        for download in &self.list {
            remove_temp(&download.temp);
        }
    }
}

pub(super) fn lock(downloads: &Mutex<Downloads>) -> std::sync::MutexGuard<'_, Downloads> {
    downloads.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn download_starting(
    url: String,
    path: &mut PathBuf,
    host: Host,
    downloads: &Arc<Mutex<Downloads>>,
    proxy: &CommandProxy,
) -> bool {
    let suggested = path.clone();
    let mut destination = Destination::Asking;
    if let Some(cb) = host.callbacks.on_download_starting {
        let (Ok(c_url), Ok(c_suggested)) = (
            CString::new(url.as_str()),
            CString::new(suggested.to_string_lossy().as_bytes()),
        ) else {
            return false;
        };
        let mut response = DownloadResponse::default();
        let allowed = crate::invoke::webview_event(|| unsafe {
            cb(host.context as *mut c_void, c_url.as_ptr(), c_suggested.as_ptr(), &mut response)
        });
        if allowed == 0 {
            return false;
        }
        if let Some(chosen) = response.destination.filter(|d| !d.is_empty()) {
            destination = Destination::Chosen(PathBuf::from(chosen));
        }
    }
    let Some(temp) = temp_download_path(&suggested) else {
        return false;
    };
    let ask = matches!(destination, Destination::Asking);
    let id = lock(downloads).add(url, temp.clone(), suggested, destination);
    if ask {
        let downloads = Arc::clone(downloads);
        let proxy_for_task = proxy.clone();
        let _ = proxy.send_event(RustinoCommand::Invoke(Task::new(move |window, _| {
            ask_destination(id, window, &downloads, host, proxy_for_task)
        })));
    }
    *path = temp;
    true
}

/// Shows the save dialog for a download, after the webview's handler returned.
fn ask_destination(
    id: u64,
    window: &tao::window::Window,
    downloads: &Mutex<Downloads>,
    host: Host,
    proxy: CommandProxy,
) {
    let suggested = lock(downloads).list.iter().find(|d| d.id == id).map(|d| d.suggested.clone());
    let Some(suggested) = suggested else {
        return;
    };
    let params = DialogParams {
        title: None,
        default_path: Some(suggested.to_string_lossy().into_owned()),
        filters: Vec::new(),
    };
    // Not locked: other downloads can start and finish while the dialog is open
    let chosen = dialogs::show_file_dialog(FileDialogKind::Save, &params, Some(window))
        .and_then(|paths| paths.into_iter().next())
        .map(PathBuf::from);
    let done = lock(downloads).choose(id, chosen);
    if let Some(download) = done {
        complete(download, host, proxy);
    }
}

/// Moves the file to its destination and reports the download to the host.
pub(super) fn complete(download: Download, host: Host, proxy: CommandProxy) {
    // Across volumes the move copies the file: not on the event loop thread
    std::thread::spawn(move || {
        let mut error = None;
        let path = match (&download.destination, download.finished) {
            (Destination::Chosen(destination), Some(true)) => match move_file(&download.temp, destination) {
                Ok(()) => Some(destination.clone()),
                Err(e) => {
                    // The downloaded file stays in its temporary folder, which is removed once the
                    // app has exited and a Rustino app starts again
                    error = Some(format!(
                        "[rustino] The download of {} could not be saved to {}: {e}. The file is kept at {} until the app exits.",
                        download.url,
                        destination.display(),
                        download.temp.display()
                    ));
                    None
                }
            },
            _ => None,
        };
        if error.is_none() {
            remove_temp(&download.temp);
        }
        let url = download.url;
        let _ = proxy.send_event(RustinoCommand::Invoke(Task::new(move |_, _| {
            if let Some(message) = &error {
                host.logger.error(message);
            }
            host.download_completed(&url, path.as_deref())
        })));
    });
}

/// Name of the temporary download folders: `rustino-download-<process id>-<random>`.
const TEMP_FOLDER_PREFIX: &str = "rustino-download-";

/// A new file in its own temporary folder, with the suggested name.
fn temp_download_path(suggested: &Path) -> Option<PathBuf> {
    static CLEANUP: std::sync::Once = std::sync::Once::new();
    let temp_dir = std::env::temp_dir();
    CLEANUP.call_once(|| {
        let temp_dir = temp_dir.clone();
        std::thread::spawn(move || remove_stale_temp_folders(&temp_dir));
    });
    let folder = create_private_folder(&temp_dir)?;
    let name = suggested.file_name().map_or_else(|| "download".into(), |n| n.to_os_string());
    Some(folder.join(name))
}

/// Creates a new folder in `parent` (often shared with the other users, like `/tmp`) that only
/// this user can open, with an unpredictable name. The creation fails if the path exists, so
/// nobody else can prepare the folder or a link in its place.
fn create_private_folder(parent: &Path) -> Option<PathBuf> {
    for _ in 0..16 {
        let name = format!("{TEMP_FOLDER_PREFIX}{}-{:016x}{:016x}", std::process::id(), random_u64(), random_u64());
        let folder = parent.join(name);
        #[allow(unused_mut)]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&folder) {
            Ok(()) => return Some(folder),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// 64 random bits: SipHash with the random keys of `RandomState`, over a counter and the time.
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    hasher.write_u128(now.map_or(0, |d| d.as_nanos()));
    hasher.finish()
}

/// The process that created a temporary download folder, from its name.
fn temp_folder_process(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(TEMP_FOLDER_PREFIX)?;
    let (pid, random) = rest.split_once('-')?;
    (random.len() == 32 && random.bytes().all(|b| b.is_ascii_hexdigit())).then_some(())?;
    pid.parse().ok()
}

/// Removes the temporary download folders of the processes that ended without removing them
/// (e.g. crashed). Only the folders of this user, never through a link.
fn remove_stale_temp_folders(temp_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(temp_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(temp_folder_process) else {
            continue;
        };
        if pid == std::process::id() || process_alive(pid) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        #[cfg(unix)]
        let mine = std::os::unix::fs::MetadataExt::uid(&metadata) == unsafe { unix::getuid() };
        // The temporary folder of Windows is the user's
        #[cfg(not(unix))]
        let mine = true;
        if metadata.is_dir() && mine {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(unix)]
mod unix {
    unsafe extern "C" {
        pub fn getuid() -> u32;
        pub fn kill(pid: i32, signal: i32) -> i32;
    }
}

/// Whether a process runs. In doubt (e.g. no permission to check) it does.
fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // Signal 0 only checks the process: EPERM means that it runs as another user
        const ESRCH: i32 = 3;
        let result = unsafe { unix::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, STILL_ACTIVE};
        use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(process) => {
                    let mut code = 0u32;
                    let exited = GetExitCodeProcess(process, &mut code).is_ok() && code != STILL_ACTIVE.0 as u32;
                    let _ = CloseHandle(process);
                    !exited
                }
                Err(e) => e.code() != ERROR_INVALID_PARAMETER.to_hresult(),
            }
        }
    }
}

fn remove_temp(temp: &Path) {
    if let Some(folder) = temp.parent() {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// Replaces an existing file: the save dialog already asked.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to).or_else(|_| copy_then_replace(from, to))
}

/// Moves a file to another volume. It's copied next to the destination, then renamed over it: a
/// copy that fails (e.g. the disk is full) leaves an existing file as it was.
fn copy_then_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let folder = match to.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let name = to.file_name().ok_or(std::io::ErrorKind::InvalidInput)?.to_string_lossy();
    let (partial, mut file) = (0..100)
        .find_map(|attempt| {
            let partial = folder.join(format!(".{name}.{}-{attempt}.part", std::process::id()));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&partial) {
                Ok(file) => Some(Ok((partial, file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .unwrap_or_else(|| Err(std::io::ErrorKind::AlreadyExists.into()))?;
    let copied = std::fs::File::open(from)
        .and_then(|mut source| std::io::copy(&mut source, &mut file))
        .and_then(|_| file.sync_all());
    drop(file);
    let replaced = copied.and_then(|()| std::fs::rename(&partial, to));
    if replaced.is_err() {
        let _ = std::fs::remove_file(&partial);
    } else {
        let _ = std::fs::remove_file(from);
    }
    replaced
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download(downloads: &mut Downloads, url: &str, destination: Destination) -> (u64, PathBuf) {
        let temp = PathBuf::from(format!("/tmp/rustino-test/{url}/file"));
        let id = downloads.add(url.into(), temp.clone(), PathBuf::from("/downloads/file"), destination);
        (id, temp)
    }

    #[test]
    fn download_waits_for_the_dialog_and_the_webview() {
        let mut downloads = Downloads::default();
        let (id, temp) = download(&mut downloads, "a", Destination::Asking);
        assert!(downloads.finish("a", Some(&temp), true).is_none(), "the dialog is still open");
        let done = downloads.choose(id, Some(PathBuf::from("/chosen"))).unwrap();
        assert!(matches!(done.destination, Destination::Chosen(ref p) if p == Path::new("/chosen")));
        assert_eq!(done.finished, Some(true));
        assert!(downloads.list.is_empty());
    }

    #[test]
    fn download_chosen_before_it_finishes() {
        let mut downloads = Downloads::default();
        let (id, _) = download(&mut downloads, "a", Destination::Asking);
        assert!(downloads.choose(id, None).is_none(), "still downloading");
        // WKWebView reports only the URL
        let done = downloads.finish("a", None, true).unwrap();
        assert!(matches!(done.destination, Destination::Canceled));
    }

    #[test]
    fn download_with_host_destination_is_done_when_finished() {
        let mut downloads = Downloads::default();
        download(&mut downloads, "a", Destination::Chosen("/x".into()));
        let (_, temp_b) = download(&mut downloads, "b", Destination::Chosen("/y".into()));
        let done = downloads.finish("ignored", Some(&temp_b), false).unwrap();
        assert_eq!(done.url, "b");
        assert_eq!(done.finished, Some(false));
        assert!(downloads.finish("unknown", None, true).is_none());
        assert_eq!(downloads.list.len(), 1);
    }

    #[test]
    fn temp_download_path_is_alone_in_its_folder() {
        let a = temp_download_path(Path::new("/downloads/report.pdf")).unwrap();
        let b = temp_download_path(Path::new("/downloads/report.pdf")).unwrap();
        assert_eq!(a.file_name().unwrap(), "report.pdf");
        assert_ne!(a.parent(), b.parent());
        assert!(!a.exists(), "the webview creates the file");
        remove_temp(&a);
        remove_temp(&b);
        assert!(!a.parent().unwrap().exists());
    }

    /// A folder of the test's own, under the system's temporary folder
    fn test_parent(name: &str) -> PathBuf {
        let parent = std::env::temp_dir().join(format!("rustino-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir(&parent).unwrap();
        parent
    }

    #[test]
    fn private_folders_are_new_and_unpredictable() {
        let parent = test_parent("private");
        let a = create_private_folder(&parent).unwrap();
        let b = create_private_folder(&parent).unwrap();
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_str().unwrap();
        assert_eq!(temp_folder_process(name), Some(std::process::id()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn temp_folder_names() {
        let random = "0123456789abcdef0123456789ABCDEF";
        assert_eq!(temp_folder_process(&format!("rustino-download-42-{random}")), Some(42));
        assert_eq!(temp_folder_process("rustino-download-42-1"), None);
        assert_eq!(temp_folder_process(&format!("rustino-download-x-{random}")), None);
        assert_eq!(temp_folder_process(&format!("other-42-{random}")), None);
        assert_eq!(temp_folder_process(&format!("rustino-download-42-{}", "g".repeat(32))), None);
    }

    #[test]
    fn stale_folders_of_ended_processes_are_removed() {
        let parent = test_parent("stale");
        let random = "0123456789abcdef0123456789abcdef";
        // No process has this id: the limits are 99999 on macOS, 2^22 on Linux, and Windows ids
        // are multiples of 4
        let stale = parent.join(format!("rustino-download-99999999-{random}"));
        let own = parent.join(format!("rustino-download-{}-{random}", std::process::id()));
        let other = parent.join("rustino-download-other");
        for folder in [&stale, &own, &other] {
            std::fs::create_dir(folder).unwrap();
            std::fs::write(folder.join("file"), "x").unwrap();
        }
        remove_stale_temp_folders(&parent);
        assert!(!stale.exists());
        assert!(own.exists() && other.exists());
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn running_processes_are_alive() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(99999999));
    }

    #[test]
    fn move_file_replaces_the_destination() {
        let from = temp_download_path(Path::new("from.txt")).unwrap();
        let to = temp_download_path(Path::new("to.txt")).unwrap();
        std::fs::write(&from, "new").unwrap();
        std::fs::write(&to, "old").unwrap();
        move_file(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "new");
        assert!(!from.exists());
        remove_temp(&from);
        remove_temp(&to);
    }

    #[test]
    fn copy_then_replace_replaces_the_destination() {
        let from = temp_download_path(Path::new("from.txt")).unwrap();
        let to = temp_download_path(Path::new("to.txt")).unwrap();
        std::fs::write(&from, "new").unwrap();
        std::fs::write(&to, "old").unwrap();
        copy_then_replace(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "new");
        assert!(!from.exists());
        // Only the destination is left in its folder: no partial copy
        assert_eq!(std::fs::read_dir(to.parent().unwrap()).unwrap().count(), 1);
        remove_temp(&from);
        remove_temp(&to);
    }

    #[test]
    fn a_failed_copy_keeps_the_destination_and_the_download() {
        let from = temp_download_path(Path::new("missing.txt")).unwrap();
        let to = temp_download_path(Path::new("to.txt")).unwrap();
        std::fs::write(&to, "old").unwrap();
        assert!(copy_then_replace(&from, &to).is_err());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "old");
        assert_eq!(std::fs::read_dir(to.parent().unwrap()).unwrap().count(), 1);
        remove_temp(&from);
        remove_temp(&to);
    }
}
