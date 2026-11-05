//! Windows capture, input, and the on-screen takeover banner.
//!
//! The banner is click-through and excluded from capture so it never appears
//! in the JPEG the phone is watching.

use super::policy::{
    absolute_mouse_axis, place_monitors, play_keys, served_size, Button, Combo, DesktopMap,
    MonitorGeometry, JPEG_QUALITY,
};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, Rgba, RgbaImage};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
            InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_NOPREFIX,
            DT_VCENTER, DT_WORDBREAK, FW_BOLD, HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{
                SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
                KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE,
                MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
                MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
                MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_APPS, VK_BACK,
                VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT,
                VK_LWIN, VK_MENU, VK_NEXT, VK_PAUSE, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT,
                VK_SPACE, VK_TAB, VK_UP,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
                GetForegroundWindow, GetSystemMetrics, PeekMessageW, RegisterClassW,
                SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
                TranslateMessage, HMENU, HWND_TOPMOST, LWA_ALPHA, MSG, PM_REMOVE, SM_CXSCREEN,
                SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
                SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE,
                WM_DESTROY, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
                WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
};

const BANNER_FAILED: u8 = 2;
const BANNER_UP: u8 = 1;

static BANNER_TEXT: Mutex<String> = Mutex::new(String::new());
static BANNER_STATE: AtomicU8 = AtomicU8::new(0);
static BANNER_EXCLUDED: AtomicBool = AtomicBool::new(false);

pub struct CapturedFrame {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub map: DesktopMap,
}

pub enum TypeStop {
    FocusChanged(usize),
    Rejected(usize),
}

pub enum InputFault {
    Rejected,
    Unexpected,
}

pub fn banner_failed() -> bool {
    BANNER_STATE.load(Ordering::SeqCst) == BANNER_FAILED
}

pub fn banner_excluded_from_capture() -> bool {
    BANNER_EXCLUDED.load(Ordering::SeqCst)
}

pub fn capture_jpeg(viewport: u32) -> Result<CapturedFrame, ()> {
    let screens = screenshots::Screen::all().map_err(|_| ())?;
    let mut images = Vec::new();
    let mut geometries = Vec::new();
    for screen in screens {
        let image = screen.capture().map_err(|_| ())?;
        let info = screen.display_info;
        let scale = if info.scale_factor.is_finite() && info.scale_factor > 0.0 {
            info.scale_factor as f64
        } else {
            1.0
        };
        geometries.push(MonitorGeometry {
            logical_x: info.x,
            logical_y: info.y,
            logical_w: if info.width == 0 {
                image.width()
            } else {
                info.width
            },
            logical_h: if info.height == 0 {
                image.height()
            } else {
                info.height
            },
            capture_w: image.width(),
            capture_h: image.height(),
            scale,
        });
        images.push(image);
    }
    let map = place_monitors(&geometries);
    if map.width == 0 || map.height == 0 || map.width > 16_000 || map.height > 16_000 {
        return Err(());
    }
    let mut canvas = RgbaImage::from_pixel(map.width, map.height, Rgba([0, 0, 0, 255]));
    for (image, placed) in images.iter().zip(map.monitors.iter()) {
        image::imageops::overlay(
            &mut canvas,
            image,
            placed.image_x as i64,
            placed.image_y as i64,
        );
    }
    let (width, height) = served_size(map.width, map.height, viewport).ok_or(())?;
    let served = if width == canvas.width() && height == canvas.height() {
        canvas
    } else {
        image::imageops::resize(
            &canvas,
            width,
            height,
            image::imageops::FilterType::Triangle,
        )
    };
    let jpeg = encode_jpeg(&served)?;
    Ok(CapturedFrame {
        jpeg,
        width,
        height,
        map,
    })
}

fn encode_jpeg(image: &RgbaImage) -> Result<Vec<u8>, ()> {
    let rgb = DynamicImage::ImageRgba8(image.clone()).into_rgb8();
    let mut bytes = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
    encoder.encode_image(&rgb).map_err(|_| ())?;
    Ok(bytes)
}

pub fn click_at(x: i32, y: i32, button: Button, double: bool) -> Result<(), InputFault> {
    let (nx, ny) = normalized(x, y).map_err(|_| InputFault::Unexpected)?;
    move_mouse(nx, ny).map_err(|_| InputFault::Rejected)?;
    let clicks = if double { 2 } else { 1 };
    for index in 0..clicks {
        if button_event(nx, ny, button, true).is_err() {
            let _ = button_event(nx, ny, button, false);
            return Err(InputFault::Rejected);
        }
        if button_event(nx, ny, button, false).is_err() {
            let _ = button_event(nx, ny, button, false);
            return Err(InputFault::Rejected);
        }
        if double && index == 0 {
            thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}

pub fn scroll_at(x: i32, y: i32, dx: i32, dy: i32) -> Result<(), InputFault> {
    let (nx, ny) = normalized(x, y).map_err(|_| InputFault::Unexpected)?;
    move_mouse(nx, ny).map_err(|_| InputFault::Rejected)?;
    if dy != 0 && wheel(nx, ny, MOUSEEVENTF_WHEEL, dy).is_err() {
        return Err(InputFault::Rejected);
    }
    if dx != 0 && wheel(nx, ny, MOUSEEVENTF_HWHEEL, dx).is_err() {
        return Err(InputFault::Rejected);
    }
    Ok(())
}

pub fn type_text(text: &str) -> Result<usize, TypeStop> {
    if send_script_active() {
        return type_chars(text);
    }
    let start = unsafe { GetForegroundWindow() };
    if start.0.is_null() {
        return Err(TypeStop::FocusChanged(0));
    }
    let mut typed = 0usize;
    for ch in text.chars() {
        let now = unsafe { GetForegroundWindow() };
        if now != start {
            return Err(TypeStop::FocusChanged(typed));
        }
        if type_one(ch).is_err() {
            return Err(TypeStop::Rejected(typed));
        }
        typed += 1;
    }
    Ok(typed)
}

fn type_chars(text: &str) -> Result<usize, TypeStop> {
    let mut typed = 0usize;
    for ch in text.chars() {
        if type_one(ch).is_err() {
            return Err(TypeStop::Rejected(typed));
        }
        typed += 1;
    }
    Ok(typed)
}

fn type_one(ch: char) -> Result<(), ()> {
    match ch {
        '\n' => tap_vk(VK_RETURN),
        '\t' => tap_vk(VK_TAB),
        _ => send_unicode(ch),
    }
}

pub fn tap_key(name: &str) -> Result<(), InputFault> {
    let sent = match name {
        "enter" => tap_vk(VK_RETURN),
        "tab" => tap_vk(VK_TAB),
        "esc" => tap_vk(VK_ESCAPE),
        "backspace" => tap_vk(VK_BACK),
        "ctrl+l" => {
            let key = virtual_key("l").ok_or(InputFault::Unexpected)?;
            modifier_tap(&[VK_CONTROL], key)
        }
        _ => return Err(InputFault::Unexpected),
    };
    sent.map_err(|_| InputFault::Rejected)
}

pub fn tap_combo(combo: &Combo) -> Result<(), InputFault> {
    let key = virtual_key(&combo.key).ok_or(InputFault::Unexpected)?;
    let mut modifiers = Vec::new();
    if combo.ctrl {
        modifiers.push(VK_CONTROL);
    }
    if combo.alt {
        modifiers.push(VK_MENU);
    }
    if combo.shift {
        modifiers.push(VK_SHIFT);
    }
    if combo.win {
        modifiers.push(VK_LWIN);
    }
    modifier_tap(&modifiers, key).map_err(|_| InputFault::Rejected)
}

pub fn run_banner(alive: impl Fn() -> bool, text: impl Fn() -> Option<String>) {
    let hwnd = match create_banner() {
        Ok(hwnd) => hwnd,
        Err(()) => {
            BANNER_STATE.store(BANNER_FAILED, Ordering::SeqCst);
            return;
        }
    };
    BANNER_STATE.store(BANNER_UP, Ordering::SeqCst);
    let mut shown = false;
    let mut previous = String::new();
    while alive() {
        drain_messages();
        match text() {
            Some(next) => {
                if next != previous {
                    if let Ok(mut guard) = BANNER_TEXT.lock() {
                        *guard = next.clone();
                    }
                    previous = next;
                    unsafe {
                        let _ = InvalidateRect(hwnd, None, BOOL(1));
                    }
                }
                if !shown {
                    show_banner(hwnd);
                    shown = true;
                }
            }
            None if shown => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                shown = false;
            }
            None => {}
        }
        thread::sleep(Duration::from_millis(200));
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = DestroyWindow(hwnd);
    }
    BANNER_EXCLUDED.store(false, Ordering::SeqCst);
    BANNER_STATE.store(0, Ordering::SeqCst);
}

fn normalized(x: i32, y: i32) -> Result<(i32, i32), ()> {
    let origin_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let origin_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let nx = absolute_mouse_axis(x, origin_x, width).ok_or(())?;
    let ny = absolute_mouse_axis(y, origin_y, height).ok_or(())?;
    Ok((nx, ny))
}

fn move_mouse(x: i32, y: i32) -> Result<(), ()> {
    send_mouse(
        x,
        y,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        0,
    )
}

fn button_event(x: i32, y: i32, button: Button, down: bool) -> Result<(), ()> {
    let flag = match (button, down) {
        (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (Button::Left, false) => MOUSEEVENTF_LEFTUP,
        (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
    };
    send_mouse(
        x,
        y,
        flag | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        0,
    )
}

fn wheel(x: i32, y: i32, flag: MOUSE_EVENT_FLAGS, notches: i32) -> Result<(), ()> {
    send_mouse(
        x,
        y,
        flag | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        (notches.saturating_mul(120)) as u32,
    )
}

fn send_mouse(x: i32, y: i32, flags: MOUSE_EVENT_FLAGS, data: u32) -> Result<(), ()> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: x,
                dy: y,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err(())
    }
}

fn tap_vk(key: VIRTUAL_KEY) -> Result<(), ()> {
    send_key(key, KEYBD_EVENT_FLAGS(0))?;
    send_key(key, KEYEVENTF_KEYUP)
}

fn modifier_tap(modifiers: &[VIRTUAL_KEY], key: VIRTUAL_KEY) -> Result<(), ()> {
    let mut strokes = Vec::with_capacity(modifiers.len() * 2 + 2);
    for modifier in modifiers {
        strokes.push((*modifier, true));
    }
    strokes.push((key, true));
    strokes.push((key, false));
    for modifier in modifiers.iter().rev() {
        strokes.push((*modifier, false));
    }
    play_keys(&strokes, |virtual_key, down| {
        let flags = if down {
            KEYBD_EVENT_FLAGS(0)
        } else {
            KEYEVENTF_KEYUP
        };
        send_key(virtual_key, flags).is_ok()
    })
}

struct SendScript {
    downs_left: usize,
    log: Vec<(u16, bool)>,
}

static SEND_SCRIPT: Mutex<Option<SendScript>> = Mutex::new(None);

#[cfg(test)]
pub fn arm_send_script(downs_before_failure: usize) {
    if let Ok(mut guard) = SEND_SCRIPT.lock() {
        *guard = Some(SendScript {
            downs_left: downs_before_failure,
            log: Vec::new(),
        });
    }
}

#[cfg(test)]
pub fn take_send_log() -> Vec<(u16, bool)> {
    SEND_SCRIPT
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
        .map(|script| script.log)
        .unwrap_or_default()
}

fn send_script_active() -> bool {
    SEND_SCRIPT
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|_| ()))
        .is_some()
}

/// `up` is true for a key-up. Returns None when no script is installed.
fn scripted_send(key: u16, up: bool) -> Option<bool> {
    let mut guard = SEND_SCRIPT.lock().ok()?;
    let script = guard.as_mut()?;
    script.log.push((key, up));
    if !up {
        if script.downs_left == 0 {
            return Some(false);
        }
        script.downs_left -= 1;
    }
    Some(true)
}

fn send_key(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> Result<(), ()> {
    let up = (flags.0 & KEYEVENTF_KEYUP.0) != 0;
    if let Some(accepted) = scripted_send(key.0, up) {
        return if accepted { Ok(()) } else { Err(()) };
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err(())
    }
}

fn send_unicode(ch: char) -> Result<(), ()> {
    let mut encoded = [0u16; 2];
    let units = ch.encode_utf16(&mut encoded);
    let strokes = units
        .iter()
        .copied()
        .flat_map(|unit| [(unit, true), (unit, false)])
        .collect::<Vec<_>>();
    play_keys(&strokes, |unit, down| {
        if let Some(accepted) = scripted_send(unit, !down) {
            return accepted;
        }
        let flags = if down {
            KEYEVENTF_UNICODE
        } else {
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
        };
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        sent == 1
    })
}

fn virtual_key(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name {
        "enter" => VK_RETURN,
        "tab" => VK_TAB,
        "esc" => VK_ESCAPE,
        "backspace" => VK_BACK,
        "delete" => VK_DELETE,
        "space" => VK_SPACE,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        "pause" => VK_PAUSE,
        "insert" => VK_INSERT,
        "apps" => VK_APPS,
        "a" => VIRTUAL_KEY(b'A' as u16),
        "b" => VIRTUAL_KEY(b'B' as u16),
        "c" => VIRTUAL_KEY(b'C' as u16),
        "d" => VIRTUAL_KEY(b'D' as u16),
        "e" => VIRTUAL_KEY(b'E' as u16),
        "f" => VIRTUAL_KEY(b'F' as u16),
        "g" => VIRTUAL_KEY(b'G' as u16),
        "h" => VIRTUAL_KEY(b'H' as u16),
        "i" => VIRTUAL_KEY(b'I' as u16),
        "j" => VIRTUAL_KEY(b'J' as u16),
        "k" => VIRTUAL_KEY(b'K' as u16),
        "l" => VIRTUAL_KEY(b'L' as u16),
        "m" => VIRTUAL_KEY(b'M' as u16),
        "n" => VIRTUAL_KEY(b'N' as u16),
        "o" => VIRTUAL_KEY(b'O' as u16),
        "p" => VIRTUAL_KEY(b'P' as u16),
        "q" => VIRTUAL_KEY(b'Q' as u16),
        "r" => VIRTUAL_KEY(b'R' as u16),
        "s" => VIRTUAL_KEY(b'S' as u16),
        "t" => VIRTUAL_KEY(b'T' as u16),
        "u" => VIRTUAL_KEY(b'U' as u16),
        "v" => VIRTUAL_KEY(b'V' as u16),
        "w" => VIRTUAL_KEY(b'W' as u16),
        "x" => VIRTUAL_KEY(b'X' as u16),
        "y" => VIRTUAL_KEY(b'Y' as u16),
        "z" => VIRTUAL_KEY(b'Z' as u16),
        "0" => VIRTUAL_KEY(b'0' as u16),
        "1" => VIRTUAL_KEY(b'1' as u16),
        "2" => VIRTUAL_KEY(b'2' as u16),
        "3" => VIRTUAL_KEY(b'3' as u16),
        "4" => VIRTUAL_KEY(b'4' as u16),
        "5" => VIRTUAL_KEY(b'5' as u16),
        "6" => VIRTUAL_KEY(b'6' as u16),
        "7" => VIRTUAL_KEY(b'7' as u16),
        "8" => VIRTUAL_KEY(b'8' as u16),
        "9" => VIRTUAL_KEY(b'9' as u16),
        other => function_virtual_key(other)?,
    })
}

fn function_virtual_key(name: &str) -> Option<VIRTUAL_KEY> {
    let number = name.strip_prefix('f')?.parse::<u16>().ok()?;
    if (1..=24).contains(&number) {
        Some(VIRTUAL_KEY(0x70 + number - 1))
    } else {
        None
    }
}

fn create_banner() -> Result<HWND, ()> {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).map_err(|_| ())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(banner_wndproc),
            hInstance: HINSTANCE(instance.0),
            hbrBackground: CreateSolidBrush(COLORREF(0x00502814)),
            lpszClassName: w!("HandsLiveScreenBanner"),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        let screen = GetSystemMetrics(SM_CXSCREEN).max(320);
        let width = 760.min(screen - 24);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            w!("HandsLiveScreenBanner"),
            w!("Live screen"),
            WS_POPUP,
            (screen - width) / 2,
            24,
            width,
            180,
            HWND::default(),
            HMENU::default(),
            HINSTANCE(instance.0),
            None,
        )
        .map_err(|_| ())?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 230, LWA_ALPHA);
        if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).is_ok() {
            BANNER_EXCLUDED.store(true, Ordering::SeqCst);
        }
        Ok(hwnd)
    }
}

fn show_banner(hwnd: HWND) {
    unsafe {
        let screen = GetSystemMetrics(SM_CXSCREEN).max(320);
        let width = 760.min(screen - 24);
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            (screen - width) / 2,
            24,
            width,
            180,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    }
}

fn drain_messages() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
            if message.message == WM_DESTROY {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn banner_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_PAINT {
        paint_banner(hwnd);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

fn paint_banner(hwnd: HWND) {
    let text = BANNER_TEXT
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let mut units = text.encode_utf16().collect::<Vec<_>>();
    if units.is_empty() {
        units.push(0x20);
    }
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut paint);
        if dc.0.is_null() {
            return;
        }
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let brush = CreateSolidBrush(COLORREF(0x00502814));
        let _ = FillRect(dc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        let font = CreateFontW(
            -32,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            PCWSTR::null(),
        );
        if !font.0.is_null() {
            let _ = SelectObject(dc, HGDIOBJ(font.0));
        }
        let _ = SetBkMode(dc, TRANSPARENT);
        let _ = SetTextColor(dc, COLORREF(0x00FFFFFF));
        let _ = DrawTextW(
            dc,
            &mut units,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_WORDBREAK | DT_NOPREFIX,
        );
        if !font.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
        let _ = EndPaint(hwnd, &paint);
    }
}
