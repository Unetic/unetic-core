#![allow(unsafe_code)]
#![allow(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions
)]

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::Arc,
};

const RTLD_NOW: c_int = 2;
const DEFAULT_LIBRARY: &str = "/usr/lib/libunetic-openwrt.so";

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
}

type HandlerFn =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_char;
type ServerNewFn = unsafe extern "C" fn(HandlerFn, *mut c_void) -> *mut c_void;
type ServerPollFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type ServerNotifyFn =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type ServerFreeFn = unsafe extern "C" fn(*mut c_void);

#[derive(Debug, Clone)]
pub struct BridgeError(pub String);

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BridgeError {}

struct Library {
    handle: *mut c_void,
}

unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Drop for Library {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                dlclose(self.handle);
            }
        }
    }
}

struct Api {
    _library: Library,
    server_new: ServerNewFn,
    server_poll: ServerPollFn,
    server_notify: ServerNotifyFn,
    server_free: ServerFreeFn,
}

#[derive(Clone)]
pub struct Bridge {
    api: Arc<Api>,
}

impl Bridge {
    pub fn load() -> Result<Self, BridgeError> {
        let path = std::env::var("UNETIC_OPENWRT_BRIDGE")
            .unwrap_or_else(|_| DEFAULT_LIBRARY.to_owned());
        Self::load_from(&path)
    }

    pub fn load_from(path: &str) -> Result<Self, BridgeError> {
        let path = CString::new(path).map_err(|error| BridgeError(error.to_string()))?;
        let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            return Err(last_dl_error("dlopen failed"));
        }
        let library = Library { handle };

        unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &str) -> Result<T, BridgeError> {
            let name = CString::new(name).map_err(|error| BridgeError(error.to_string()))?;
            unsafe {
                dlerror();
                let pointer = dlsym(handle, name.as_ptr());
                if pointer.is_null() {
                    return Err(last_dl_error("dlsym failed"));
                }
                Ok(std::mem::transmute_copy(&pointer))
            }
        }

        let api = unsafe {
            Api {
                server_new: symbol(library.handle, "unetic_ubus_server_new")?,
                server_poll: symbol(library.handle, "unetic_ubus_server_poll")?,
                server_notify: symbol(library.handle, "unetic_ubus_server_notify")?,
                server_free: symbol(library.handle, "unetic_ubus_server_free")?,
                _library: library,
            }
        };
        Ok(Self { api: Arc::new(api) })
    }

    pub fn server<F>(&self, handler: F) -> Result<Server, BridgeError>
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
    {
        let callback = Box::new(Callback {
            handler: Arc::new(handler),
        });
        let callback_ptr = Box::into_raw(callback);
        let handle = unsafe {
            (self.api.server_new)(callback_trampoline, callback_ptr.cast::<c_void>())
        };
        if handle.is_null() {
            unsafe {
                drop(Box::from_raw(callback_ptr));
            }
            return Err(BridgeError("failed to register unetic ubus object".into()));
        }
        Ok(Server {
            api: self.api.clone(),
            handle,
            callback: callback_ptr,
        })
    }
}

type Handler = dyn Fn(&str, &str) -> String + Send + Sync;

struct Callback {
    handler: Arc<Handler>,
}

pub struct Server {
    api: Arc<Api>,
    handle: *mut c_void,
    callback: *mut Callback,
}

impl Server {
    pub fn poll(&mut self, timeout_ms: i32) -> Result<(), BridgeError> {
        let rc = unsafe { (self.api.server_poll)(self.handle, timeout_ms) };
        if rc == 0 {
            Ok(())
        } else {
            Err(BridgeError(format!("ubus poll failed with status {rc}")))
        }
    }

    pub fn notify(&mut self, event: &str, json: &str) -> Result<(), BridgeError> {
        let event = cstring(event)?;
        let json = cstring(json)?;
        let rc = unsafe {
            (self.api.server_notify)(self.handle, event.as_ptr(), json.as_ptr())
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(BridgeError(format!("ubus notify failed with status {rc}")))
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            (self.api.server_free)(self.handle);
            drop(Box::from_raw(self.callback));
        }
    }
}

unsafe extern "C" fn callback_trampoline(
    userdata: *mut c_void,
    method: *const c_char,
    request: *const c_char,
) -> *mut c_char {
    if userdata.is_null() || method.is_null() {
        return ptr::null_mut();
    }

    let callback = unsafe { &*userdata.cast::<Callback>() };
    let method = unsafe { CStr::from_ptr(method) }.to_string_lossy();
    let request = if request.is_null() {
        "{}".into()
    } else {
        unsafe { CStr::from_ptr(request) }.to_string_lossy()
    };

    let Ok(response) = catch_unwind(AssertUnwindSafe(|| (callback.handler)(&method, &request)))
    else {
        return ptr::null_mut();
    };
    copy_to_c_heap(&response)
}

fn copy_to_c_heap(value: &str) -> *mut c_char {
    let bytes = value.as_bytes();
    let out = unsafe { malloc(bytes.len() + 1) }.cast::<u8>();
    if out.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
        out.add(bytes.len()).write(0);
    }
    out.cast::<c_char>()
}

fn cstring(value: &str) -> Result<CString, BridgeError> {
    CString::new(value).map_err(|error| BridgeError(error.to_string()))
}

fn last_dl_error(prefix: &str) -> BridgeError {
    let pointer = unsafe { dlerror() };
    if pointer.is_null() {
        return BridgeError(prefix.to_owned());
    }
    let message = unsafe { CStr::from_ptr(pointer) }.to_string_lossy();
    BridgeError(format!("{prefix}: {message}"))
}
