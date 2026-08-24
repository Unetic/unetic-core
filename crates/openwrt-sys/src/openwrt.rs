mod ffi;

use std::{
    ffi::{CStr, CString, c_char, c_int},
    fmt,
    mem::zeroed,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{Arc, Mutex, OnceLock},
};

use ffi::*;

type Handler = dyn Fn(&str, &str) -> String + Send + Sync;

#[derive(Debug, Clone)]
pub struct BridgeError(pub String);

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BridgeError {}

pub struct Bridge;

impl Bridge {
    pub fn load() -> Result<Self, BridgeError> {
        Ok(Self)
    }

    pub fn server<F>(&self, methods: &[&str], handler: F) -> Result<Server, BridgeError>
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
    {
        let handler = Arc::new(handler) as Arc<Handler>;
        install_handler(Arc::clone(&handler))?;

        let server = Server::new(methods, handler);
        if let Err(error) = server {
            clear_handler();
            return Err(error);
        }
        Ok(server.expect("successful server construction"))
    }
}

pub struct Server {
    context: *mut UbusContext,
    object: Box<UbusObject>,
    object_type: Box<UbusObjectType>,
    _object_name: CString,
    _method_names: Vec<CString>,
    _methods: Vec<UbusMethod>,
    _handler: Arc<Handler>,
}

unsafe impl Send for Server {}

impl Server {
    fn new(methods: &[&str], handler: Arc<Handler>) -> Result<Self, BridgeError> {
        if methods.is_empty() {
            return Err(BridgeError("at least one ubus method is required".into()));
        }

        let object_name = cstring("unetic")?;
        let method_names = methods
            .iter()
            .map(|method| cstring(method))
            .collect::<Result<Vec<_>, _>>()?;
        let method_defs = method_names
            .iter()
            .map(|name| UbusMethod {
                name: name.as_ptr(),
                handler: method_handler,
                mask: 0,
                tags: 0,
                policy: ptr::null(),
                policy_count: 0,
            })
            .collect::<Vec<_>>();

        let mut server = Self {
            context: ptr::null_mut(),
            object: Box::new(unsafe { zeroed() }),
            object_type: Box::new(UbusObjectType {
                name: object_name.as_ptr(),
                id: 0,
                methods: method_defs.as_ptr(),
                method_count: method_defs
                    .len()
                    .try_into()
                    .map_err(|_| BridgeError("too many ubus methods".into()))?,
            }),
            _object_name: object_name,
            _method_names: method_names,
            _methods: method_defs,
            _handler: handler,
        };

        unsafe {
            if uloop_init() != UBUS_STATUS_OK {
                return Err(BridgeError("failed to initialize uloop".into()));
            }

            server.context = ubus_connect(ptr::null());
            if server.context.is_null() {
                uloop_done();
                return Err(BridgeError("failed to connect to ubus".into()));
            }

            server.object.name = server._object_name.as_ptr();
            server.object.object_type = server.object_type.as_mut();
            server.object.methods = server._methods.as_ptr();
            server.object.method_count = server._methods.len().try_into().unwrap_or(c_int::MAX);

            let status = ubus_add_object(server.context, server.object.as_mut());
            if status != UBUS_STATUS_OK {
                ubus_free(server.context);
                uloop_done();
                return Err(BridgeError(format!(
                    "failed to register ubus object 'unetic': {status}"
                )));
            }

            let status = uloop_fd_add(&mut (*server.context).sock, ULOOP_BLOCKING_READ);
            if status != UBUS_STATUS_OK {
                ubus_remove_object(server.context, server.object.as_mut());
                ubus_free(server.context);
                uloop_done();
                return Err(BridgeError(format!(
                    "failed to add ubus socket to uloop: {status}"
                )));
            }
        }

        Ok(server)
    }

    pub fn poll(&mut self, timeout_ms: i32) -> Result<(), BridgeError> {
        let timeout_ms = if timeout_ms < 0 { 100 } else { timeout_ms };
        let status = unsafe {
            uloop_cancelled = false;
            uloop_run_timeout(timeout_ms)
        };
        if status == UBUS_STATUS_OK {
            Ok(())
        } else {
            Err(BridgeError(format!(
                "ubus poll failed with status {status}"
            )))
        }
    }

    pub fn notify(&mut self, event: &str, json: &str) -> Result<(), BridgeError> {
        let event = cstring(event)?;
        let json = cstring(json)?;
        let mut message = unsafe { zeroed::<BlobBuf>() };

        let status = unsafe {
            blob_buf_init(&mut message, BLOBMSG_TYPE_TABLE);
            if !blobmsg_add_json_from_string(&mut message, json.as_ptr()) {
                blob_buf_free(&mut message);
                return Err(BridgeError(
                    "failed to encode ubus notification JSON".into(),
                ));
            }
            let status = ubus_notify(
                self.context,
                self.object.as_mut(),
                event.as_ptr(),
                message.head,
                -1,
            );
            blob_buf_free(&mut message);
            status
        };

        if status == UBUS_STATUS_OK {
            Ok(())
        } else {
            Err(BridgeError(format!(
                "ubus notify failed with status {status}"
            )))
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            ubus_remove_object(self.context, self.object.as_mut());
            ubus_free(self.context);
            uloop_done();
        }
        clear_handler();
    }
}

fn handler_slot() -> &'static Mutex<Option<Arc<Handler>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<Handler>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn install_handler(handler: Arc<Handler>) -> Result<(), BridgeError> {
    let mut slot = handler_slot()
        .lock()
        .map_err(|_| BridgeError("ubus handler lock is poisoned".into()))?;
    if slot.is_some() {
        return Err(BridgeError("only one ubus server can run at a time".into()));
    }
    *slot = Some(handler);
    Ok(())
}

fn clear_handler() {
    if let Ok(mut slot) = handler_slot().lock() {
        *slot = None;
    }
}

unsafe extern "C" fn method_handler(
    context: *mut UbusContext,
    _object: *mut UbusObject,
    request: *mut UbusRequestData,
    method: *const c_char,
    message: *mut BlobAttr,
) -> c_int {
    if context.is_null() || request.is_null() || method.is_null() {
        return UBUS_STATUS_INVALID_ARGUMENT;
    }

    let method = unsafe { CStr::from_ptr(method) }.to_string_lossy();
    let request_json = request_json(message);
    let Some(request_json) = request_json else {
        return UBUS_STATUS_UNKNOWN_ERROR;
    };

    let response = handler_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(Arc::clone))
        .and_then(|handler| {
            catch_unwind(AssertUnwindSafe(|| handler(&method, &request_json))).ok()
        });
    let Some(response) = response else {
        return UBUS_STATUS_UNKNOWN_ERROR;
    };

    send_reply(context, request, &response)
}

fn request_json(message: *mut BlobAttr) -> Option<String> {
    if message.is_null() {
        return Some("{}".into());
    }

    let json =
        unsafe { blobmsg_format_json_with_cb(message, true, None, std::ptr::null_mut(), -1) };
    if json.is_null() {
        return None;
    }
    let value = unsafe { CStr::from_ptr(json) }
        .to_string_lossy()
        .into_owned();
    unsafe { libc::free(json.cast()) };
    Some(value)
}

fn send_reply(context: *mut UbusContext, request: *mut UbusRequestData, response: &str) -> c_int {
    let Ok(response) = cstring(response) else {
        return UBUS_STATUS_UNKNOWN_ERROR;
    };
    let mut reply = unsafe { zeroed::<BlobBuf>() };

    unsafe {
        blob_buf_init(&mut reply, BLOBMSG_TYPE_TABLE);
        if !blobmsg_add_json_from_string(&mut reply, response.as_ptr()) {
            blob_buf_free(&mut reply);
            return UBUS_STATUS_UNKNOWN_ERROR;
        }
        let status = ubus_send_reply(context, request, reply.head);
        blob_buf_free(&mut reply);
        status
    }
}

fn cstring(value: &str) -> Result<CString, BridgeError> {
    CString::new(value).map_err(|error| BridgeError(error.to_string()))
}
