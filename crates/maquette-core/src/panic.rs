//! Panic capture for plugins. A panic inside a wasm plugin surfaces in the
//! host as a bare trap; with the hook installed, the message and location are
//! kept so the host can fetch them afterwards through the `get_last_panic`
//! export that [`panic_export!`](crate::panic_export) defines.

use std::cell::RefCell;
use std::sync::Once;

thread_local! {
    static LAST_PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

static HOOK: Once = Once::new();

/// Record every later panic's location and message. Idempotent and cheap, so
/// plugins call it at each entry point.
pub fn install_hook() {
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let loc = info.location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "?".into());
            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() { (*s).to_string() }
                else if let Some(s) = info.payload().downcast_ref::<String>() { s.clone() }
                else { "(non-string panic payload)".into() };
            LAST_PANIC.with(|p| *p.borrow_mut() = format!("panic at {}: {}", loc, msg));
        }));
    });
}

/// The last captured panic message, empty when none fired.
pub fn last_panic() -> String {
    LAST_PANIC.with(|p| p.borrow().clone())
}

/// Define the plugin export `get_last_panic`, returning the last captured
/// panic message (empty when none). Invoke once at the plugin's crate root;
/// the crate must depend on `wasm_minimal_protocol`.
#[macro_export]
macro_rules! panic_export {
    () => {
        #[wasm_minimal_protocol::wasm_func]
        fn get_last_panic() -> Vec<u8> {
            $crate::panic::install_hook();
            $crate::panic::last_panic().into_bytes()
        }
    };
}
