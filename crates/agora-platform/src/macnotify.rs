//! macOS half of [`crate::desktop_notify`]: the app's own notification through `UNUserNotificationCenter` (ADR-0048 decision 5), so
//! it shows under the app's name and a click brings the app forward. Called through the Objective-C runtime: no crate.
//!
//! - Only inside an app bundle. `+[UNUserNotificationCenter currentNotificationCenter]` throws (and ends the process) in a program
//!   that is not one (measured on macOS 27.2: `bundleProxyForCurrentProcess is nil`), so without a bundle identifier nothing is
//!   called and the answer is `Unsupported`. A program run from a terminal (`cargo run`, the CLI) is never one.
//! - Permission: the first call asks the user (the system's own question); the answer is awaited for at most 30 s. Denied, or
//!   turned off later in System Settings > Notifications: `PermissionDenied`, nothing shown.
//! - The callback the permission request needs is an Objective-C block. It is a *global* block built by hand (no captures, never
//!   copied or freed by the runtime, with the layout of the Block ABI); its only effect is to store the answer in a static.
//! - The words travel as `NSString` objects, never inside a format or a script.

use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

type Id = *mut c_void;
type Sel = *mut c_void;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn class_getClassMethod(cls: Id, sel: Sel) -> *mut c_void;
    fn class_getInstanceMethod(cls: Id, sel: Sel) -> *mut c_void;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[link(name = "UserNotifications", kind = "framework")]
extern "C" {}

// The isa of every global block (libSystem's libclosure).
extern "C" {
    static _NSConcreteGlobalBlock: c_void;
}

/// `UNAuthorizationOptionAlert`.
const OPTION_ALERT: c_ulong = 1 << 2;
/// `BLOCK_IS_GLOBAL`: the runtime never copies, retains or frees it.
const BLOCK_IS_GLOBAL: c_int = 1 << 28;

/// The permission answer: 0 waiting, 1 granted, 2 denied.
static ANSWER: AtomicU8 = AtomicU8::new(0);

#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
}

/// The Block ABI's literal layout for a block without captures.
#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const Block, i8, Id),
    descriptor: *const BlockDescriptor,
}

// SAFETY: the block is immutable static data; the runtime only reads it and calls `invoke`, which touches an atomic.
unsafe impl Sync for Block {}

static DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<Block>() as c_ulong,
};

/// `^(BOOL granted, NSError *error)`: stores the answer.
unsafe extern "C" fn answered(_block: *const Block, granted: i8, _error: Id) {
    ANSWER.store(if granted != 0 { 1 } else { 2 }, Ordering::SeqCst);
}

static ON_ANSWER: Block = Block {
    // Only the address of the runtime's static is taken.
    isa: std::ptr::addr_of!(_NSConcreteGlobalBlock),
    flags: BLOCK_IS_GLOBAL,
    reserved: 0,
    invoke: answered,
    descriptor: &DESCRIPTOR,
};

/// `objc_msgSend` as a function of the given signature.
macro_rules! send {
    ($ty:ty) => {
        // SAFETY (at each use): the receiver and selector were checked to exist, and `$ty` is that method's exact C signature.
        unsafe { std::mem::transmute::<unsafe extern "C" fn(), $ty>(objc_msgSend as unsafe extern "C" fn()) }
    };
}

fn class(name: &std::ffi::CStr) -> Option<Id> {
    // SAFETY: a NUL-terminated name.
    let c = unsafe { objc_getClass(name.as_ptr()) };
    (!c.is_null()).then_some(c)
}

fn sel(name: &std::ffi::CStr) -> Sel {
    // SAFETY: a NUL-terminated name; selectors are interned for the life of the process.
    unsafe { sel_registerName(name.as_ptr()) }
}

fn has_class_method(cls: Id, s: Sel) -> bool {
    // SAFETY: `cls` is a class from `objc_getClass`.
    !unsafe { class_getClassMethod(cls, s) }.is_null()
}

fn has_method(cls: Id, s: Sel) -> bool {
    // SAFETY: `cls` is a class from `objc_getClass`.
    !unsafe { class_getInstanceMethod(cls, s) }.is_null()
}

/// An autoreleased `NSString` with `text`.
fn ns_string(text: &str) -> Option<Id> {
    let c = std::ffi::CString::new(text).ok()?;
    let cls = class(c"NSString")?;
    let s = sel(c"stringWithUTF8String:");
    if !has_class_method(cls, s) {
        return None;
    }
    let f = send!(unsafe extern "C" fn(Id, Sel, *const c_char) -> Id);
    // SAFETY: see `send!`; `c` lives for the call.
    let out = unsafe { f(cls, s, c.as_ptr()) };
    (!out.is_null()).then_some(out)
}

/// Whether this process runs from an app bundle (its main bundle has an identifier).
fn in_bundle() -> bool {
    let (Some(cls), s, id) = (
        class(c"NSBundle"),
        sel(c"mainBundle"),
        sel(c"bundleIdentifier"),
    ) else {
        return false;
    };
    if !has_class_method(cls, s) || !has_method(cls, id) {
        return false;
    }
    let main = send!(unsafe extern "C" fn(Id, Sel) -> Id);
    // SAFETY: see `send!`.
    let bundle = unsafe { main(cls, s) };
    if bundle.is_null() {
        return false;
    }
    let ident = send!(unsafe extern "C" fn(Id, Sel) -> Id);
    // SAFETY: see `send!`; `bundle` is an NSBundle.
    !unsafe { ident(bundle, id) }.is_null()
}

pub(crate) fn desktop_notify(title: &str, body: &str) -> io::Result<()> {
    let unsupported = || io::Error::from(io::ErrorKind::Unsupported);
    if !in_bundle() {
        return Err(unsupported());
    }
    // SAFETY: balanced with the pop below; nothing between them unwinds (no panicking code).
    let pool = unsafe { objc_autoreleasePoolPush() };
    let result = post(title, body);
    // SAFETY: the pool pushed above.
    unsafe { objc_autoreleasePoolPop(pool) };
    result
}

fn post(title: &str, body: &str) -> io::Result<()> {
    let unsupported = || io::Error::from(io::ErrorKind::Unsupported);
    let center_cls = class(c"UNUserNotificationCenter").ok_or_else(unsupported)?;
    let content_cls = class(c"UNMutableNotificationContent").ok_or_else(unsupported)?;
    let request_cls = class(c"UNNotificationRequest").ok_or_else(unsupported)?;
    let current = sel(c"currentNotificationCenter");
    let authorize = sel(c"requestAuthorizationWithOptions:completionHandler:");
    let add = sel(c"addNotificationRequest:withCompletionHandler:");
    let new = sel(c"new");
    let set_title = sel(c"setTitle:");
    let set_body = sel(c"setBody:");
    let make = sel(c"requestWithIdentifier:content:trigger:");
    let release = sel(c"release");
    if !has_class_method(center_cls, current)
        || !has_method(center_cls, authorize)
        || !has_method(center_cls, add)
        || !has_class_method(content_cls, new)
        || !has_method(content_cls, set_title)
        || !has_method(content_cls, set_body)
        || !has_class_method(request_cls, make)
    {
        return Err(unsupported());
    }
    let get = send!(unsafe extern "C" fn(Id, Sel) -> Id);
    // SAFETY: see `send!`; inside a bundle (checked by the caller), so this does not throw.
    let center = unsafe { get(center_cls, current) };
    if center.is_null() {
        return Err(unsupported());
    }
    // Asked once per call; after the first answer the system answers at once without asking again.
    ANSWER.store(0, Ordering::SeqCst);
    let ask = send!(unsafe extern "C" fn(Id, Sel, c_ulong, *const Block));
    // SAFETY: see `send!`; `ON_ANSWER` is a static global block.
    unsafe { ask(center, authorize, OPTION_ALERT, &ON_ANSWER) };
    let asked = Instant::now();
    while ANSWER.load(Ordering::SeqCst) == 0 && asked.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(50));
    }
    if ANSWER.load(Ordering::SeqCst) != 1 {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    let (Some(t), Some(b), Some(ident)) = (
        ns_string(title),
        ns_string(body),
        ns_string("agora-approvals"),
    ) else {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    };
    let make_content = send!(unsafe extern "C" fn(Id, Sel) -> Id);
    // SAFETY: see `send!`; `new` returns an owned object, released below.
    let content = unsafe { make_content(content_cls, new) };
    if content.is_null() {
        return Err(io::Error::other("UNMutableNotificationContent"));
    }
    let set = send!(unsafe extern "C" fn(Id, Sel, Id));
    // SAFETY: see `send!`; `content` is a UNMutableNotificationContent, `t` and `b` NSStrings.
    unsafe {
        set(content, set_title, t);
        set(content, set_body, b);
    }
    let request = send!(unsafe extern "C" fn(Id, Sel, Id, Id, Id) -> Id);
    // SAFETY: see `send!`; a nil trigger means "now". The request is autoreleased.
    let req = unsafe { request(request_cls, make, ident, content, std::ptr::null_mut()) };
    let drop_content = send!(unsafe extern "C" fn(Id, Sel));
    // SAFETY: `content` was made by `new` above and is no longer used here (the request keeps its own copy).
    unsafe { drop_content(content, release) };
    if req.is_null() {
        return Err(io::Error::other("UNNotificationRequest"));
    }
    let post = send!(unsafe extern "C" fn(Id, Sel, Id, *const c_void));
    // SAFETY: see `send!`; a nil completion handler is allowed.
    unsafe { post(center, add, req, std::ptr::null()) };
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    fn a_program_that_is_not_an_app_bundle_never_calls_the_notification_center() {
        // A test binary has no bundle identifier: the call that would end the process is never made.
        assert!(!super::in_bundle());
        assert_eq!(
            super::desktop_notify("Agora", "test").unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn the_block_has_the_layout_of_the_block_abi() {
        // isa, flags, reserved, invoke, descriptor: 8 + 4 + 4 + 8 + 8 on 64-bit macOS.
        assert_eq!(std::mem::size_of::<super::Block>(), 32);
        assert_eq!(super::DESCRIPTOR.size, 32);
    }
}
