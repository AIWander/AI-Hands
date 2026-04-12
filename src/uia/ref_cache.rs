use std::{cell::RefCell, collections::HashMap};

use serde::Serialize;
use uia_lib::{Point, Rect};
use windows::Win32::UI::Accessibility::IUIAutomationElement;

#[derive(Clone, Serialize)]
pub struct CachedElementMeta {
    #[serde(rename = "ref")]
    pub ref_id: String,
    pub name: String,
    pub control_type: String,
    pub class_name: String,
    pub automation_id: String,
    pub bounding_rect: Rect,
    pub is_enabled: bool,
    pub is_visible: bool,
    pub center: Point,
}

#[derive(Clone)]
pub struct CachedElement {
    pub element: IUIAutomationElement,
    pub meta: CachedElementMeta,
}

struct SnapshotCache {
    window_title: String,
    hwnd_label: String,
    refs: HashMap<String, CachedElement>,
}

thread_local! {
    static REF_CACHE: RefCell<Option<SnapshotCache>> = const { RefCell::new(None) };
}

pub fn store_snapshot(window_title: String, hwnd_label: String, refs: HashMap<String, CachedElement>) {
    REF_CACHE.with(|cache| {
        *cache.borrow_mut() = Some(SnapshotCache {
            window_title,
            hwnd_label,
            refs,
        });
    });
}

pub fn resolve_ref(ref_id: &str) -> Result<CachedElement, String> {
    REF_CACHE.with(|cache| {
        let guard = cache.borrow();
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| "No UIA snapshot cached. Call uia_get_state first.".to_string())?;

        snapshot.refs.get(ref_id).cloned().ok_or_else(|| {
            format!(
                "Ref '{}' not found in cached UIA snapshot ({} @ {}). Take a fresh uia_get_state snapshot.",
                ref_id, snapshot.window_title, snapshot.hwnd_label
            )
        })
    })
}
