//! A tap of the fn (Globe) key alone toggles composer dictation. A webview never sees fn, so an
//! AppKit local monitor watches the app's own key events and tells the UI with `dictation://toggle`.

use tauri::AppHandle;

const EVENT_TOGGLE: &str = "dictation://toggle";
/// fn held longer than this is a hold, not a tap.
const TAP_SECONDS: f64 = 0.6;

/// A key event, reduced to what a tap depends on.
#[derive(Clone, Copy)]
enum Key {
    /// A modifier changed: whether fn is down now, whether any of ⇧⌃⌥⌘ is, and when, in seconds.
    Flags { function: bool, others: bool, at: f64 },
    /// Any key went down. Arrows and F-keys carry the fn flag on their key down; they never count.
    Down,
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
enum Tap {
    #[default]
    Idle,
    /// fn went down alone at this time.
    Down(f64),
    /// Something else joined in; nothing counts until every modifier is up again.
    Spoiled,
}

impl Tap {
    /// The next state, and whether this event completed a tap.
    fn feed(self, key: Key) -> (Tap, bool) {
        match key {
            Key::Down if self == Tap::Idle => (Tap::Idle, false),
            Key::Down => (Tap::Spoiled, false),
            Key::Flags { function: false, others: false, at } => (Tap::Idle, matches!(self, Tap::Down(down) if at - down <= TAP_SECONDS)),
            Key::Flags { function: true, others: false, at } if self == Tap::Idle => (Tap::Down(at), false),
            Key::Flags { function: true, others: false, .. } => (self, false),
            Key::Flags { .. } => (Tap::Spoiled, false),
        }
    }
}

#[cfg(target_os = "macos")]
pub fn watch(app: &AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
    use std::cell::Cell;
    use std::ptr::NonNull;
    use tauri::Emitter;

    let app = app.clone();
    let tap = Cell::new(Tap::Idle);
    let others = NSEventModifierFlags::Shift | NSEventModifierFlags::Control | NSEventModifierFlags::Option | NSEventModifierFlags::Command;
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let seen = unsafe { event.as_ref() };
        let key = if seen.r#type() == NSEventType::FlagsChanged {
            let flags = seen.modifierFlags();
            Key::Flags { function: flags.contains(NSEventModifierFlags::Function), others: flags.intersects(others), at: seen.timestamp() }
        } else {
            Key::Down
        };
        let (next, tapped) = tap.get().feed(key);
        tap.set(next);
        if tapped {
            let _ = app.emit(EVENT_TOGGLE, ());
        }
        event.as_ptr()
    });
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::FlagsChanged | NSEventMask::KeyDown, &block) };
    std::mem::forget(monitor);
}

#[cfg(not(target_os = "macos"))]
pub fn watch(_: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(keys: &[Key]) -> Vec<bool> {
        let mut tap = Tap::Idle;
        keys.iter()
            .map(|&key| {
                let (next, tapped) = tap.feed(key);
                tap = next;
                tapped
            })
            .collect()
    }

    const fn flags(function: bool, others: bool, at: f64) -> Key {
        Key::Flags { function, others, at }
    }

    #[test]
    fn fn_down_and_up_alone_is_a_tap() {
        assert_eq!(run(&[flags(true, false, 10.0), flags(false, false, 10.2)]), [false, true]);
    }

    #[test]
    fn two_taps_toggle_twice() {
        let keys = [flags(true, false, 1.0), flags(false, false, 1.1), flags(true, false, 2.0), flags(false, false, 2.1)];
        assert_eq!(run(&keys), [false, true, false, true]);
    }

    #[test]
    fn fn_with_an_arrow_is_not_a_tap() {
        assert_eq!(run(&[flags(true, false, 1.0), Key::Down, flags(false, false, 1.2)]), [false, false, false]);
    }

    #[test]
    fn an_arrow_alone_is_not_a_tap() {
        assert_eq!(run(&[Key::Down, Key::Down]), [false, false]);
    }

    #[test]
    fn fn_held_too_long_is_not_a_tap() {
        assert_eq!(run(&[flags(true, false, 1.0), flags(false, false, 1.7)]), [false, false]);
    }

    #[test]
    fn fn_with_shift_is_not_a_tap_even_when_shift_lets_go_first() {
        let keys = [flags(true, false, 1.0), flags(true, true, 1.1), flags(true, false, 1.2), flags(false, false, 1.3)];
        assert_eq!(run(&keys), [false, false, false, false]);
    }

    #[test]
    fn fn_pressed_while_shift_is_held_is_not_a_tap() {
        let keys = [flags(false, true, 1.0), flags(true, true, 1.1), flags(false, true, 1.2), flags(false, false, 1.3)];
        assert_eq!(run(&keys), [false, false, false, false]);
    }

    #[test]
    fn a_spoiled_press_does_not_spoil_the_next_tap() {
        let keys = [flags(true, false, 1.0), Key::Down, flags(false, false, 1.2), flags(true, false, 2.0), flags(false, false, 2.1)];
        assert_eq!(run(&keys), [false, false, false, false, true]);
    }
}
