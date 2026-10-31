use std::sync::{Arc, Mutex};
use std::thread;

/// Larger than any screen: the image would take gigabytes
pub const MAX_SIZE: u32 = 16384;

pub struct SplashWindow {
    thread_handle: Option<thread::JoinHandle<()>>,
    close_sender: Arc<Mutex<Option<std::sync::mpsc::Sender<()>>>>,
}

impl SplashWindow {
    /// Fails on macOS and Linux, where the splashscreen isn't implemented: their windows need the
    /// main thread, which the app's event loop takes.
    pub fn new(image_path: &str, width: u32, height: u32) -> Result<Self, String> {
        if !cfg!(target_os = "windows") {
            return Err("The splashscreen is only supported on Windows".into());
        }
        if width == 0 || height == 0 || width > MAX_SIZE || height > MAX_SIZE {
            return Err(format!("Invalid splash size {width}x{height}"));
        }
        let image_data = std::fs::read(image_path)
            .map_err(|e| format!("Failed to read image file: {}", e))?;

        let img = image::load_from_memory(&image_data)
            .map_err(|e| format!("Failed to decode image: {}", e))?
            .resize_exact(width, height, image::imageops::FilterType::Lanczos3)
            .to_rgba8();

        let (close_tx, close_rx) = std::sync::mpsc::channel::<()>();
        let close_sender = Arc::new(Mutex::new(Some(close_tx)));

        let handle = thread::spawn(move || {
            #[cfg(target_os = "windows")]
            windows_splash::run(img, width, height, close_rx);

            #[cfg(not(target_os = "windows"))]
            let _ = (img, close_rx);
        });

        Ok(SplashWindow {
            thread_handle: Some(handle),
            close_sender,
        })
    }

    pub fn close(&self) {
        if let Ok(mut sender) = self.close_sender.lock()
            && let Some(tx) = sender.take()
        {
            let _ = tx.send(());
        }
    }
}

impl Drop for SplashWindow {
    fn drop(&mut self) {
        self.close();
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

/// A pixel as `UpdateLayeredWindow` takes it: BGRA, with the colors premultiplied by the alpha.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn premultiplied_bgra([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let premultiply = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
    [premultiply(b), premultiply(g), premultiply(r), a]
}

#[cfg(test)]
mod tests {
    use super::premultiplied_bgra;

    #[test]
    fn transparent_pixels_are_black_and_opaque_ones_keep_their_color() {
        assert_eq!(premultiplied_bgra([88, 101, 242, 0]), [0, 0, 0, 0]);
        assert_eq!(premultiplied_bgra([88, 101, 242, 255]), [242, 101, 88, 255]);
        assert_eq!(premultiplied_bgra([255, 255, 255, 128]), [128, 128, 128, 128]);
    }
}

#[cfg(target_os = "windows")]
mod windows_splash {
    use super::premultiplied_bgra;
    use image::RgbaImage;
    use std::sync::mpsc::Receiver;
    use windows::core::*;
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn run(img: RgbaImage, width: u32, height: u32, close_rx: Receiver<()>) {
        unsafe {
            let class_name = w!("RustinoSplash");
            let instance: HINSTANCE = std::mem::zeroed();

            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let screen_h = GetSystemMetrics(SM_CYSCREEN);
            let x = (screen_w - width as i32) / 2;
            let y = (screen_h - height as i32) / 2;

            // Layered: the image's alpha shows the desktop through the transparent parts
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                class_name,
                w!(""),
                WS_POPUP,
                x,
                y,
                width as i32,
                height as i32,
                None,
                None,
                Some(instance),
                None,
            );
            let Ok(hwnd) = hwnd else {
                let _ = UnregisterClassW(class_name, Some(instance));
                return;
            };

            show_image(hwnd, &img, width, height, POINT { x, y });
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

            let mut msg = MSG::default();
            loop {
                if close_rx.try_recv().is_ok() {
                    break;
                }

                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        return;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                std::thread::sleep(std::time::Duration::from_millis(16));
            }

            let _ = DestroyWindow(hwnd);
            let _ = UnregisterClassW(class_name, Some(instance));
        }
    }

    /// Gives the image to the layered window, which the system composes with what's below it.
    unsafe fn show_image(hwnd: HWND, img: &RgbaImage, width: u32, height: u32, position: POINT) {
        unsafe {
            let screen = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen));

            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0,
                    ..Default::default()
                },
                ..Default::default()
            };

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(screen), &bi, DIB_RGB_COLORS, &mut bits, None, 0)
                .unwrap_or_default();

            if !bits.is_null() {
                // At most MAX_SIZE² pixels: no overflow
                let len = width as usize * height as usize * 4;
                let dst = std::slice::from_raw_parts_mut(bits as *mut u8, len);
                for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(img.pixels()) {
                    dst_px.copy_from_slice(&premultiplied_bgra(src_px.0));
                }
            }

            let old = SelectObject(mem_dc, bitmap.into());
            let size = SIZE {
                cx: width as i32,
                cy: height as i32,
            };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                Some(screen),
                Some(&position),
                Some(&size),
                Some(mem_dc),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );

            SelectObject(mem_dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen);
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            match msg {
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }
}
