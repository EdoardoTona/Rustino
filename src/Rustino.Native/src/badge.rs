//! The count on the icon of the app: an overlay on the taskbar button (Windows), the Dock badge
//! (macOS), the launcher entry (Linux).

use std::panic::catch_unwind;

use crate::commands::RustinoCommand;
use crate::window::RustinoWindow;

/// Queued until the window runs. 0 or less removes the badge.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_badge_count(
    instance: *mut RustinoWindow,
    count: i32,
    bg_r: u8,
    bg_g: u8,
    bg_b: u8,
    fg_r: u8,
    fg_g: u8,
    fg_b: u8,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let badge = if count > 0 { Some(count as u32) } else { None };
            inst.send_command(RustinoCommand::SetBadgeCount {
                count: badge,
                bg_r,
                bg_g,
                bg_b,
                fg_r,
                fg_g,
                fg_b,
            });
        }
    });
}

pub(crate) fn set_badge_count(_window: &tao::window::Window, count: Option<u32>, _bg: [u8; 3], _fg: [u8; 3]) {
    #[cfg(target_os = "windows")]
    {
        set_badge_count_windows(_window, count, _bg, _fg);
    }
    #[cfg(target_os = "macos")]
    {
        set_badge_count_macos(count);
    }
    #[cfg(target_os = "linux")]
    {
        crate::window_ext::launcher::set_count(count);
    }
}

#[cfg(target_os = "windows")]
fn set_badge_count_windows(window: &tao::window::Window, count: Option<u32>, bg: [u8; 3], fg: [u8; 3]) {
    use tao::platform::windows::WindowExtWindows;
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::HWND;

    unsafe {
        let Ok(taskbar): Result<ITaskbarList3, _> =
            CoCreateInstance(&TaskbarList, None, CLSCTX_ALL)
        else {
            return;
        };

        let hwnd = HWND(window.hwnd() as *mut std::ffi::c_void);

        match count {
            None | Some(0) => {
                let _ = taskbar.SetOverlayIcon(hwnd, HICON::default(), None);
            }
            Some(n) => {
                if let Some(icon) = create_badge_icon(n, bg, fg) {
                    // Accessible description of the overlay, read by Narrator
                    let description = windows::core::HSTRING::from(n.to_string());
                    let _ = taskbar.SetOverlayIcon(hwnd, icon, &description);
                    let _ = DestroyIcon(icon);
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn create_badge_icon(count: u32, bg: [u8; 3], fg: [u8; 3]) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::core::w;

    let size: i32 = 32;

    unsafe {
        let hdc_screen = GetDC(None);
        let hdc = CreateCompatibleDC(Some(hdc_screen));
        ReleaseDC(None, hdc_screen);

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -(size);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;

        // --- Pass 1: Render white text on black to get coverage mask ---
        let mut text_bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let text_bmp = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut text_bits, None, 0).ok()?;
        let old_bmp = SelectObject(hdc, text_bmp.into());

        let text_pixels = text_bits as *mut u8;
        std::ptr::write_bytes(text_pixels, 0, (size * size * 4) as usize);

        let text = if count > 99 { "99+".to_string() } else { count.to_string() };
        let font_size = if text.len() <= 1 { 22 } else if text.len() == 2 { 18 } else { 14 };
        let font = CreateFontW(
            -font_size, 0, 0, 0,
            FW_BOLD.0 as i32,
            0, 0, 0,
            FONT_CHARSET(0),
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32 | FF_SWISS.0 as u32,
            w!("Segoe UI"),
        );
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));

        let wide_text: Vec<u16> = text.encode_utf16().collect();
        let nudge_x: i32 = 0;
        let nudge_y: i32 = -1;
        let mut rc = windows::Win32::Foundation::RECT {
            left: nudge_x, top: nudge_y, right: size + nudge_x, bottom: size + nudge_y,
        };
        DrawTextW(
            hdc,
            &mut wide_text.as_slice().to_vec(),
            &mut rc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP,
        );

        // Read text coverage from red channel (white text on black = coverage in any channel)
        let mut text_mask = vec![0u8; (size * size) as usize];
        for (i, item) in text_mask.iter_mut().enumerate().take((size * size) as usize) {
            *item = *text_pixels.add(i * 4 + 2); // R channel
        }

        SelectObject(hdc, old_font);
        SelectObject(hdc, old_bmp);
        let _ = DeleteObject(text_bmp.into());
        let _ = DeleteObject(font.into());

        // --- Pass 2: Compose final icon ---
        let mut final_bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let final_bmp = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut final_bits, None, 0).ok()?;

        let pixels = final_bits as *mut u8;
        std::ptr::write_bytes(pixels, 0, (size * size * 4) as usize);

        let cx = size as f32 / 2.0;
        let cy = size as f32 / 2.0;
        let r = cx - 0.5;
        // Sub-pixel shift for single digits (GDI centers on cell, not glyph)
        let text_shift_x: f32 = if text.len() == 1 { 1.0 } else { 0.0 };

        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= r {
                    let circle_alpha = (r - dist).min(1.0);

                    // Sample text mask with sub-pixel offset (bilinear interpolation)
                    let sx = x as f32 - text_shift_x;
                    let sy = y as f32;
                    let sx0 = sx.floor() as i32;
                    let sy0 = sy.floor() as i32;
                    let fx = sx - sx.floor();
                    let fy = sy - sy.floor();
                    let sample = |px: i32, py: i32| -> f32 {
                        if px >= 0 && px < size && py >= 0 && py < size {
                            text_mask[(py * size + px) as usize] as f32
                        } else {
                            0.0
                        }
                    };
                    let c00 = sample(sx0, sy0);
                    let c10 = sample(sx0 + 1, sy0);
                    let c01 = sample(sx0, sy0 + 1);
                    let c11 = sample(sx0 + 1, sy0 + 1);
                    let coverage = (c00 * (1.0 - fx) * (1.0 - fy)
                        + c10 * fx * (1.0 - fy)
                        + c01 * (1.0 - fx) * fy
                        + c11 * fx * fy) / 255.0;

                    // Blend: text foreground over background, then apply circle alpha
                    let out_r = fg[0] as f32 * coverage + bg[0] as f32 * (1.0 - coverage);
                    let out_g = fg[1] as f32 * coverage + bg[1] as f32 * (1.0 - coverage);
                    let out_b = fg[2] as f32 * coverage + bg[2] as f32 * (1.0 - coverage);
                    let alpha = (circle_alpha * 255.0) as u8;

                    let i = (y * size + x) as usize;
                    let idx = i * 4;
                    let p = pixels.add(idx);
                    // Premultiplied BGRA
                    *p = ((out_b * circle_alpha) as u8).min(alpha);
                    *p.add(1) = ((out_g * circle_alpha) as u8).min(alpha);
                    *p.add(2) = ((out_r * circle_alpha) as u8).min(alpha);
                    *p.add(3) = alpha;
                }
            }
        }

        let mask = CreateBitmap(size, size, 1, 1, None);
        let ii = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: final_bmp,
        };
        let icon = CreateIconIndirect(&ii).ok();

        let _ = DeleteObject(final_bmp.into());
        let _ = DeleteObject(mask.into());
        let _ = DeleteDC(hdc);

        icon
    }
}

#[cfg(target_os = "macos")]
fn set_badge_count_macos(count: Option<u32>) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let dock_tile = app.dockTile();
    match count {
        None | Some(0) => {
            dock_tile.setBadgeLabel(Some(&NSString::from_str("")));
        }
        Some(n) => {
            dock_tile.setBadgeLabel(Some(&NSString::from_str(&n.to_string())));
        }
    }
}
