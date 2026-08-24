use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

pub(super) const UBUS_STATUS_OK: c_int = 0;
pub(super) const UBUS_STATUS_UNKNOWN_ERROR: c_int = 2;
pub(super) const UBUS_STATUS_INVALID_ARGUMENT: c_int = 3;
pub(super) const ULOOP_BLOCKING_READ: c_uint = 9;
pub(super) const BLOBMSG_TYPE_TABLE: c_int = 2;

pub(super) type UbusHandler = unsafe extern "C" fn(
    *mut UbusContext,
    *mut UbusObject,
    *mut UbusRequestData,
    *const c_char,
    *mut BlobAttr,
) -> c_int;

#[repr(C)]
pub(super) struct ListHead {
    next: *mut ListHead,
    prev: *mut ListHead,
}

#[repr(C)]
pub(super) struct AvlNode {
    list: ListHead,
    parent: *mut AvlNode,
    left: *mut AvlNode,
    right: *mut AvlNode,
    key: *const c_void,
    balance: i8,
    leader: bool,
}

#[repr(C)]
pub(super) struct AvlTree {
    list_head: ListHead,
    root: *mut AvlNode,
    count: c_uint,
    allow_dups: bool,
    comp: *const c_void,
    cmp_ptr: *mut c_void,
}

#[repr(C)]
pub(super) struct UloopFd {
    callback: *const c_void,
    fd: c_int,
    eof: bool,
    error: bool,
    registered: bool,
    flags: u8,
}

#[repr(C)]
pub(super) struct UloopTimeout {
    list: ListHead,
    pending: bool,
    callback: *const c_void,
    time: Timeval,
}

#[repr(C)]
pub(super) struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

#[repr(C)]
pub(super) struct UbusContext {
    requests: ListHead,
    objects: AvlTree,
    pending: ListHead,
    pub(super) sock: UloopFd,
    pending_timer: UloopTimeout,
}

#[repr(C)]
pub(super) struct BlobAttr {
    _id_len: u32,
}

#[repr(C)]
pub(super) struct BlobBuf {
    pub(super) head: *mut BlobAttr,
    grow: *const c_void,
    buflen: c_int,
    buf: *mut c_void,
}

#[repr(C)]
pub(super) struct UbusMethod {
    pub(super) name: *const c_char,
    pub(super) handler: UbusHandler,
    pub(super) mask: c_ulong,
    pub(super) tags: c_ulong,
    pub(super) policy: *const c_void,
    pub(super) policy_count: c_int,
}

#[repr(C)]
pub(super) struct UbusObjectType {
    pub(super) name: *const c_char,
    pub(super) id: u32,
    pub(super) methods: *const UbusMethod,
    pub(super) method_count: c_int,
}

#[repr(C)]
pub(super) struct UbusObject {
    avl: AvlNode,
    pub(super) name: *const c_char,
    id: u32,
    path: *const c_char,
    pub(super) object_type: *mut UbusObjectType,
    subscribe_callback: *const c_void,
    has_subscribers: bool,
    pub(super) methods: *const UbusMethod,
    pub(super) method_count: c_int,
}

#[repr(C)]
pub(super) struct UbusRequestData {
    _private: [u8; 0],
}

#[link(name = "ubus")]
unsafe extern "C" {
    pub(super) fn ubus_connect(path: *const c_char) -> *mut UbusContext;
    pub(super) fn ubus_free(context: *mut UbusContext);
    pub(super) fn ubus_add_object(context: *mut UbusContext, object: *mut UbusObject) -> c_int;
    pub(super) fn ubus_remove_object(context: *mut UbusContext, object: *mut UbusObject) -> c_int;
    pub(super) fn ubus_send_reply(
        context: *mut UbusContext,
        request: *mut UbusRequestData,
        message: *mut BlobAttr,
    ) -> c_int;
    pub(super) fn ubus_notify(
        context: *mut UbusContext,
        object: *mut UbusObject,
        event: *const c_char,
        message: *mut BlobAttr,
        timeout: c_int,
    ) -> c_int;
}

#[link(name = "ubox")]
unsafe extern "C" {
    pub(super) fn uloop_init() -> c_int;
    pub(super) fn uloop_done();
    pub(super) fn uloop_fd_add(socket: *mut UloopFd, flags: c_uint) -> c_int;
    pub(super) fn uloop_run_timeout(timeout_ms: c_int) -> c_int;
    pub(super) static mut uloop_cancelled: bool;
    pub(super) fn blob_buf_init(buffer: *mut BlobBuf, id: c_int) -> c_int;
    pub(super) fn blob_buf_free(buffer: *mut BlobBuf);
}

#[link(name = "blobmsg_json")]
unsafe extern "C" {
    pub(super) fn blobmsg_add_json_from_string(buffer: *mut BlobBuf, json: *const c_char) -> bool;
    pub(super) fn blobmsg_format_json_with_cb(
        message: *mut BlobAttr,
        list: bool,
        callback: Option<unsafe extern "C" fn(*mut c_void, *mut BlobAttr) -> *const c_char>,
        private: *mut c_void,
        indent: c_int,
    ) -> *mut c_char;
}
