#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libubox/blobmsg.h>
#include <libubox/blobmsg_json.h>
#include <libubox/uloop.h>
#include <libubox/utils.h>
#include <libubus.h>

typedef char *(*unetic_handler_fn)(void *userdata, const char *method,
                                    const char *request_json);

struct unetic_server {
    struct ubus_context *ctx;
    struct ubus_object object;
    struct ubus_object_type type;
    unetic_handler_fn handler;
    void *userdata;
};

static int unetic_method_handler(struct ubus_context *ctx,
                                 struct ubus_object *obj,
                                 struct ubus_request_data *req,
                                 const char *method,
                                 struct blob_attr *msg)
{
    struct unetic_server *server = container_of(obj, struct unetic_server, object);
    struct blob_buf reply = {};
    char *request_json = NULL;
    char *response_json = NULL;
    int rc = UBUS_STATUS_OK;

    if (msg)
        request_json = blobmsg_format_json(msg, true);

    response_json = server->handler(server->userdata, method,
                                    request_json ? request_json : "{}");
    free(request_json);

    if (!response_json)
        return UBUS_STATUS_UNKNOWN_ERROR;

    blob_buf_init(&reply, 0);
    if (!blobmsg_add_json_from_string(&reply, response_json)) {
        free(response_json);
        blob_buf_free(&reply);
        return UBUS_STATUS_UNKNOWN_ERROR;
    }

    free(response_json);
    rc = ubus_send_reply(ctx, req, reply.head);
    blob_buf_free(&reply);
    return rc;
}

enum {
    SET_SSID_SSID,
    SET_SSID_EXPECTED_REVISION,
    SET_SSID_REQUEST_ID,
    __SET_SSID_MAX,
};

static const struct blobmsg_policy set_ssid_policy[__SET_SSID_MAX] = {
    [SET_SSID_SSID] = { .name = "ssid", .type = BLOBMSG_TYPE_STRING },
    [SET_SSID_EXPECTED_REVISION] = { .name = "expected_revision", .type = BLOBMSG_TYPE_INT64 },
    [SET_SSID_REQUEST_ID] = { .name = "request_id", .type = BLOBMSG_TYPE_STRING },
};

enum {
    MAINTENANCE_REASON,
    __MAINTENANCE_MAX,
};

static const struct blobmsg_policy maintenance_policy[__MAINTENANCE_MAX] = {
    [MAINTENANCE_REASON] = { .name = "reason", .type = BLOBMSG_TYPE_STRING },
};

static const struct ubus_method unetic_methods[] = {
    UBUS_METHOD_NOARG("state", unetic_method_handler),
    UBUS_METHOD_NOARG("wifi.get", unetic_method_handler),
    UBUS_METHOD("wifi.set_ssid", unetic_method_handler, set_ssid_policy),
    UBUS_METHOD_NOARG("operation.get", unetic_method_handler),
    UBUS_METHOD_NOARG("maintenance.get", unetic_method_handler),
    UBUS_METHOD("maintenance.enter", unetic_method_handler, maintenance_policy),
    UBUS_METHOD_NOARG("maintenance.exit", unetic_method_handler),
    UBUS_METHOD_NOARG("health.get", unetic_method_handler),
};

void *unetic_ubus_server_new(unetic_handler_fn handler, void *userdata)
{
    struct unetic_server *server;
    int rc;

    if (!handler)
        return NULL;

    server = calloc(1, sizeof(*server));
    if (!server)
        return NULL;

    if (uloop_init() != 0) {
        free(server);
        return NULL;
    }

    server->ctx = ubus_connect(NULL);
    if (!server->ctx) {
        uloop_done();
        free(server);
        return NULL;
    }

    server->handler = handler;
    server->userdata = userdata;

    server->type.name = "unetic";
    server->type.methods = unetic_methods;
    server->type.n_methods = ARRAY_SIZE(unetic_methods);

    server->object.name = "unetic";
    server->object.type = &server->type;
    server->object.methods = unetic_methods;
    server->object.n_methods = ARRAY_SIZE(unetic_methods);

    ubus_add_uloop(server->ctx);
    rc = ubus_add_object(server->ctx, &server->object);
    if (rc != UBUS_STATUS_OK) {
        ubus_free(server->ctx);
        uloop_done();
        free(server);
        return NULL;
    }

    return server;
}

int unetic_ubus_server_poll(void *handle, int timeout_ms)
{
    struct unetic_server *server = handle;

    if (!server)
        return UBUS_STATUS_INVALID_ARGUMENT;

    if (timeout_ms < 0)
        timeout_ms = 100;

    uloop_run_timeout(timeout_ms);
    return UBUS_STATUS_OK;
}

int unetic_ubus_server_notify(void *handle, const char *event,
                              const char *json)
{
    struct unetic_server *server = handle;
    struct blob_buf message = {};
    int rc;

    if (!server || !event || !json)
        return UBUS_STATUS_INVALID_ARGUMENT;

    blob_buf_init(&message, 0);
    if (!blobmsg_add_json_from_string(&message, json)) {
        blob_buf_free(&message);
        return UBUS_STATUS_INVALID_ARGUMENT;
    }

    rc = ubus_notify(server->ctx, &server->object, event, message.head, -1);
    blob_buf_free(&message);
    return rc;
}

void unetic_ubus_server_free(void *handle)
{
    struct unetic_server *server = handle;

    if (!server)
        return;

    ubus_remove_object(server->ctx, &server->object);
    ubus_free(server->ctx);
    uloop_done();
    free(server);
}
