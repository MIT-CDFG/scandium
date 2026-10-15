/* SPDX-License-Identifier: Apache-2.0
 * Copyright (c) 2026 the Appname contributors
 *
 * C ABI smoke test (plan §2.2 "C smoke test in gate"; ES Appendix N.1
 * black-box discipline): drive the engine from plain C99 through
 * include/appname.h only — create a document handle, apply a real doc.new
 * envelope through the single mutation door (raw bytes + length, WD-1),
 * read back composite pixels, verify a typed structured rejection, undo,
 * poll the §9.5 event queue, close, and prove lifecycle misuse
 * (double-close) is a typed error. Exit 0 on success. Built with the
 * system C compiler against the committed header + the staticlib and
 * executed by crates/appname-cabi/tests/c_smoke.rs on every
 * `cargo test --workspace`.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "appname.h"

static int failures = 0;

static void check(int ok, const char *what) {
    if (ok) {
        printf("ok  %s\n", what);
    } else {
        printf("FAIL %s\n", what);
        failures++;
    }
}

/* Scrape the first `"id":"<uuid>"` out of compact JSON (36-char values;
 * "docId" cannot match — its `id` is not quote-preceded). */
static int scrape_first_id(const char *json, char out[37]) {
    const char *p = strstr(json, "\"id\":\"");
    if (!p) return 0;
    p += 6;
    if (strlen(p) < 36) return 0;
    memcpy(out, p, 36);
    out[36] = '\0';
    return 1;
}

/* Build a 3-sample appname.samples.v1 blob: "APNSAMPL" + version 1 +
 * flags 0 + count (u32) header, then 48-byte records (x,y f64; pressure,
 * tilt_x, tilt_y, rotation, velocity f32; 4 reserved zero bytes; t f64),
 * all little-endian — this host (arm64) is little-endian. The stroke line
 * crosses (60, 40). */
static size_t build_samples_blob(uint8_t *out) {
    static const double xs[3] = { 20.0, 60.0, 100.0 };
    static const double ys[3] = { 20.0, 40.0, 60.0 };
    static const double ts[3] = { 0.0, 0.04, 0.08 };
    uint16_t version = 1, flags = 0;
    uint32_t count = 3;
    uint8_t *p = out;
    int i;
    memcpy(p, "APNSAMPL", 8); p += 8;
    memcpy(p, &version, 2); p += 2;
    memcpy(p, &flags, 2); p += 2;
    memcpy(p, &count, 4); p += 4;
    for (i = 0; i < 3; i++) {
        float one = 1.0f, zero = 0.0f;
        memcpy(p, &xs[i], 8); p += 8;
        memcpy(p, &ys[i], 8); p += 8;
        memcpy(p, &one, 4); p += 4;   /* pressure */
        memcpy(p, &zero, 4); p += 4;  /* tilt_x */
        memcpy(p, &zero, 4); p += 4;  /* tilt_y */
        memcpy(p, &zero, 4); p += 4;  /* rotation */
        memcpy(p, &zero, 4); p += 4;  /* velocity */
        memset(p, 0, 4); p += 4;      /* reserved (canonical zero) */
        memcpy(p, &ts[i], 8); p += 8;
    }
    return (size_t)(p - out);
}

static void print_error(const char *what, AppnameError *err) {
    if (err) {
        size_t n = 0;
        const char *json = appname_error_json(err, &n);
        printf("%s error (status %d): %s\n", what, (int)appname_error_status(err),
               json ? json : "(no payload)");
    } else {
        printf("%s error: (no error object)\n", what);
    }
}

int main(void) {
    /* 0. Version, callable before anything (ES §9.1 rule 6). */
    AppnameVersion v = appname_version();
    printf("abi %u.%u engine %u.%u\n", v.abi_major, v.abi_minor, v.engine_major, v.engine_minor);
    /* 1.7 = the §19.1 block: info emits `imageRect` and `doc.crop` honours
       `deletePixels:false`. Pinned EXACTLY, not `>=`: this assertion's job is
       to notice a bump, and it did — leave it exact so the next one is noticed
       too. (`font_door.rs` uses `>=` deliberately, because it asserts "the 1.6
       block ships", which stays true forever after 1.6.) */
    check(v.abi_major == 1 && v.abi_minor == 8, "version reports ABI 1.8");

    /* 1. Create a document handle (pre-init shell). */
    AppnameDoc *doc = NULL;
    AppnameError *err = NULL;
    AppnameStatus st = appname_doc_new(&doc, &err);
    check(st == APPNAME_OK && doc != NULL && err == NULL, "appname_doc_new");
    if (st != APPNAME_OK) { print_error("doc_new", err); appname_error_free(err); return 1; }

    /* 2. THE single mutation entry: a doc.new envelope as raw bytes. */
    const char *envelope =
        "{\"schema\":\"appname.command.v1\","
        "\"id\":\"5a5e1f7e-57a2-4bcd-9e0c-123456789abc\","
        "\"kind\":\"doc.new\","
        "\"params\":{\"width\":64,\"height\":48,\"background\":\"white\"}}";
    AppnameString *receipt = NULL;
    st = appname_doc_apply_command(doc, (const uint8_t *)envelope, strlen(envelope),
                                   &receipt, &err);
    check(st == APPNAME_OK && receipt != NULL && err == NULL, "apply doc.new returns APPNAME_OK");
    if (st != APPNAME_OK) { print_error("apply", err); appname_error_free(err); return 1; }
    size_t rlen = 0;
    const char *rjson = appname_string_data(receipt, &rlen);
    printf("receipt: %s\n", rjson ? rjson : "(null)");
    check(rjson != NULL && rlen > 0 && strlen(rjson) == rlen,
          "receipt length matches NUL-terminated data");
    check(rjson != NULL && strstr(rjson, "\"ok\":true") != NULL
              && strstr(rjson, "\"kind\":\"doc.new\"") != NULL,
          "receipt carries ok:true and the kind");
    check(appname_string_free(receipt) == APPNAME_OK, "receipt freed");

    /* 3. Composite readback: full canvas, RGBA8 straight sRGB. */
    AppnameRect rect = { 0, 0, 64, 48 };
    AppnameImageBuffer *buf = NULL;
    st = appname_doc_read_region(doc, rect, &buf, &err);
    check(st == APPNAME_OK && buf != NULL, "appname_doc_read_region");
    if (st != APPNAME_OK) { print_error("read_region", err); appname_error_free(err); return 1; }
    AppnameMapInfo map;
    memset(&map, 0, sizeof map);
    st = appname_buffer_map(buf, &map);
    check(st == APPNAME_OK && map.data != NULL, "appname_buffer_map");
    check(map.width == 64 && map.height == 48 && map.stride == 64 * 4
              && map.len == (size_t)64 * 48 * 4,
          "map geometry (64x48, tight stride)");
    check(map.pixel_format == 0 && map.color_model == 3 && map.premultiplied == 0,
          "map encoding is straight RGBA8");
    if (map.data) {
        const uint8_t *p00 = map.data;                                /* (0,0)   */
        const uint8_t *pxy = map.data + 47 * map.stride + 63 * 4;     /* (63,47) */
        printf("pixel(0,0) = %u,%u,%u,%u\n", p00[0], p00[1], p00[2], p00[3]);
        check(p00[0] == 255 && p00[1] == 255 && p00[2] == 255 && p00[3] == 255,
              "white background reads back 255,255,255,255 at (0,0)");
        check(pxy[0] == 255 && pxy[1] == 255 && pxy[2] == 255 && pxy[3] == 255,
              "white background reads back 255,255,255,255 at (63,47)");
    }
    check(appname_buffer_unmap(buf) == APPNAME_OK, "appname_buffer_unmap");
    check(appname_buffer_free(buf) == APPNAME_OK, "appname_buffer_free");

    /* 3b. Samples ingress (§9.2 asset block) + painting through the ABI. */
    uint8_t garbage[8] = { 1, 2, 3, 4, 5, 6, 7, 8 };
    AppnameString *asset_id = NULL;
    st = appname_asset_create(doc, garbage, sizeof garbage, &asset_id, &err);
    check(st == APPNAME_ERR_INVALID_ARG && asset_id == NULL,
          "garbage blob rejects typed at ingress");
    if (err) {
        size_t elen = 0;
        const char *ejson = appname_error_json(err, &elen);
        check(ejson && strstr(ejson, "error.command.bad_target") != NULL
                  && strstr(ejson, "appname.samples.v1") != NULL,
              "ingress reject names the blob contract");
        appname_error_free(err);
        err = NULL;
    } else {
        check(0, "ingress reject populates the error out-param");
    }

    uint8_t blob[16 + 3 * 48];
    size_t blob_len = build_samples_blob(blob);
    check(blob_len == sizeof blob, "blob layout is 16 + 3*48 bytes");
    st = appname_asset_create(doc, blob, blob_len, &asset_id, &err);
    check(st == APPNAME_OK && asset_id != NULL, "appname_asset_create");
    if (st != APPNAME_OK) { print_error("asset_create", err); appname_error_free(err); return 1; }
    size_t alen = 0;
    const char *auuid = appname_string_data(asset_id, &alen);
    printf("assetId: %s\n", auuid ? auuid : "(null)");
    check(auuid != NULL && alen == 36, "assetId is a 36-char uuid string");

    /* Layer id for the stroke target, scraped from the info JSON. */
    AppnameString *info = NULL;
    st = appname_doc_info_json(doc, &info, &err);
    check(st == APPNAME_OK && info != NULL, "appname_doc_info_json");
    char layer_id[37];
    const char *ijson = info ? appname_string_data(info, NULL) : NULL;
    check(ijson != NULL && scrape_first_id(ijson, layer_id),
          "layer id scraped from info JSON");
    check(appname_string_free(info) == APPNAME_OK, "info freed");

    /* brush.stroke referencing the registered asset (built-in hard-round-16
     * preset id — deterministic v5 constant, pinned by the conformance
     * suite). */
    char stroke[640];
    snprintf(stroke, sizeof stroke,
             "{\"schema\":\"appname.command.v1\","
             "\"id\":\"5a5e1f7e-57a2-4bcd-9e0c-223456789abc\","
             "\"kind\":\"brush.stroke\","
             "\"target\":{\"layerId\":\"%s\"},"
             "\"params\":{\"brushPreset\":\"eeced2e4-ee37-587c-a8a0-8fca734089d1\","
             "\"color\":{\"model\":\"rgb\",\"comps\":[1,0,0,0]},\"seed\":7},"
             "\"samples\":\"%s\"}",
             layer_id, auuid);
    check(appname_string_free(asset_id) == APPNAME_OK, "assetId freed");
    AppnameString *stroke_receipt = NULL;
    st = appname_doc_apply_command(doc, (const uint8_t *)stroke, strlen(stroke),
                                   &stroke_receipt, &err);
    check(st == APPNAME_OK && stroke_receipt != NULL, "brush.stroke applies via the ABI");
    if (st != APPNAME_OK) { print_error("stroke", err); appname_error_free(err); return 1; }
    const char *sjson = appname_string_data(stroke_receipt, NULL);
    printf("stroke receipt: %s\n", sjson ? sjson : "(null)");
    check(sjson && strstr(sjson, "\"kind\":\"brush.stroke\"") != NULL
              && strstr(sjson, "\"dirty\":[") != NULL,
          "stroke receipt carries kind + dirty rects");
    check(appname_string_free(stroke_receipt) == APPNAME_OK, "stroke receipt freed");

    /* Painted readback: the stroke crosses (60, 40) — red over white. */
    AppnameRect center = { 60, 40, 1, 1 };
    AppnameImageBuffer *paint = NULL;
    st = appname_doc_read_region(doc, center, &paint, &err);
    check(st == APPNAME_OK && paint != NULL, "read_region at the stroke center");
    AppnameMapInfo pmap;
    memset(&pmap, 0, sizeof pmap);
    check(appname_buffer_map(paint, &pmap) == APPNAME_OK && pmap.data != NULL,
          "painted pixel mapped");
    if (pmap.data) {
        printf("pixel(60,40) = %u,%u,%u,%u\n",
               pmap.data[0], pmap.data[1], pmap.data[2], pmap.data[3]);
        check(pmap.data[0] > 229 && pmap.data[1] < 153 && pmap.data[3] == 255,
              "readback shows the red stroke (not the white background)");
    }
    check(appname_buffer_free(paint) == APPNAME_OK, "painted buffer freed");

    /* 4. Typed structured rejection through the same door. */
    const char *bad =
        "{\"schema\":\"appname.command.v1\","
        "\"id\":\"6b6e2f8f-68b3-4cde-8f1d-23456789abcd\","
        "\"kind\":\"no.such_kind\"}";
    AppnameString *rejection = NULL;
    st = appname_doc_apply_command(doc, (const uint8_t *)bad, strlen(bad), &rejection, &err);
    check(st == APPNAME_ERR_UNSUPPORTED, "unknown kind maps to APPNAME_ERR_UNSUPPORTED");
    check(err != NULL, "rejection populates the error out-param");
    if (err) {
        size_t elen = 0;
        const char *ejson = appname_error_json(err, &elen);
        printf("rejection error: %s\n", ejson ? ejson : "(null)");
        check(ejson && strstr(ejson, "error.command.unknown_kind") != NULL,
              "error payload carries the taxonomy code");
        check(appname_error_status(err) == APPNAME_ERR_UNSUPPORTED, "stored status matches");
        check(appname_error_free(err) == APPNAME_OK, "error freed");
        err = NULL;
    }
    if (rejection) {
        const char *wire = appname_string_data(rejection, NULL);
        printf("rejection wire: %s\n", wire ? wire : "(null)");
        check(wire && strstr(wire, "\"ok\":false") != NULL
                  && strstr(wire, "6b6e2f8f-68b3-4cde-8f1d-23456789abcd") != NULL,
              "rejection wire document quotes the envelope id");
        check(appname_string_free(rejection) == APPNAME_OK, "rejection wire freed");
    } else {
        check(0, "rejection wire document present");
    }

    /* 5. Undo (engine function): reverts the stroke via COW deltas. */
    AppnameString *row = NULL;
    st = appname_doc_undo(doc, &row, &err);
    check(st == APPNAME_OK && row != NULL, "appname_doc_undo");
    if (st != APPNAME_OK) { print_error("undo", err); appname_error_free(err); return 1; }
    const char *rowjson = appname_string_data(row, NULL);
    printf("undo row: %s\n", rowjson ? rowjson : "(null)");
    check(rowjson && strstr(rowjson, "\"kind\":\"brush.stroke\"") != NULL
              && strstr(rowjson, "\"active\":false") != NULL,
          "undo row names the reverted stroke");
    check(appname_string_free(row) == APPNAME_OK, "undo row freed");

    /* The undone pixel is white again. */
    paint = NULL;
    st = appname_doc_read_region(doc, center, &paint, &err);
    check(st == APPNAME_OK && appname_buffer_map(paint, &pmap) == APPNAME_OK,
          "post-undo readback");
    if (pmap.data) {
        check(pmap.data[0] == 255 && pmap.data[1] == 255 && pmap.data[2] == 255,
              "undo restored the white background at (60,40)");
    }
    check(appname_buffer_free(paint) == APPNAME_OK, "post-undo buffer freed");

    /* 6. Poll events: apply + undo queued dirtyRects + history pairs. */
    AppnameString *events = NULL;
    st = appname_doc_poll_events(doc, &events, &err);
    check(st == APPNAME_OK && events != NULL, "appname_doc_poll_events");
    const char *evjson = events ? appname_string_data(events, NULL) : NULL;
    printf("events: %s\n", evjson ? evjson : "(null)");
    check(evjson && strstr(evjson, "appname.event.dirtyRects.v1") != NULL,
          "events carry dirtyRects payloads");
    check(evjson && strstr(evjson, "appname.event.history.v1") != NULL,
          "events carry history payloads");
    check(evjson && strstr(evjson, "\"thread\":\"command\"") != NULL,
          "payloads declare their thread (§9.5)");
    check(appname_string_free(events) == APPNAME_OK, "events freed");

    /* Second poll drains to the empty array. */
    st = appname_doc_poll_events(doc, &events, &err);
    evjson = (st == APPNAME_OK && events) ? appname_string_data(events, NULL) : NULL;
    check(evjson && strcmp(evjson, "[]") == 0, "second poll drains to []");
    check(appname_string_free(events) == APPNAME_OK, "empty events freed");

    /* 6b. Selection readback (ABI 1.2 block): rect select, Gray8 coverage. */
    const char *select_rect =
        "{\"schema\":\"appname.command.v1\","
        "\"id\":\"5a5e1f7e-57a2-4bcd-9e0c-323456789abc\","
        "\"kind\":\"selection.set\","
        "\"params\":{\"source\":\"rect\",\"rect\":{\"x\":8,\"y\":4,\"w\":12,\"h\":6}}}";
    st = appname_doc_apply_command(doc, (const uint8_t *)select_rect, strlen(select_rect),
                                   NULL, &err);
    check(st == APPNAME_OK, "selection.set applies via the ABI");
    if (st != APPNAME_OK) { print_error("selection.set", err); appname_error_free(err); return 1; }
    AppnameRect selrect = { 0, 0, 64, 48 };
    AppnameImageBuffer *sel = NULL;
    st = appname_doc_read_selection(doc, selrect, &sel, &err);
    check(st == APPNAME_OK && sel != NULL, "appname_doc_read_selection");
    if (st != APPNAME_OK) { print_error("read_selection", err); appname_error_free(err); return 1; }
    AppnameMapInfo smap;
    memset(&smap, 0, sizeof smap);
    check(appname_buffer_map(sel, &smap) == APPNAME_OK && smap.data != NULL,
          "selection buffer mapped");
    check(smap.pixel_format == 0 && smap.color_model == 0 && smap.premultiplied == 0,
          "selection map encoding is straight Gray8");
    check(smap.stride == 64 && smap.len == (size_t)64 * 48, "Gray8 tight stride = width");
    if (smap.data) {
        check(smap.data[4 * 64 + 8] == 255, "selected pixel (8,4) reads 255");
        check(smap.data[0] == 0, "unselected pixel (0,0) reads 0");
    }
    check(appname_buffer_free(sel) == APPNAME_OK, "selection buffer freed");

    /* 6c. Export door (ABI 1.2 block): engine-side PNG + a typed reject. */
    const char *scratch = getenv("CABI_SMOKE_DIR");
    if (!scratch) scratch = ".";
    char out_png[1024];
    snprintf(out_png, sizeof out_png, "%s/smoke_export.png", scratch);
    st = appname_doc_export(doc, out_png, "png", NULL, &err);
    check(st == APPNAME_OK, "appname_doc_export png");
    if (st != APPNAME_OK) { print_error("export", err); appname_error_free(err); err = NULL; }
    st = appname_doc_export(doc, out_png, "tga", NULL, &err);
    check(st == APPNAME_ERR_UNSUPPORTED, "unknown export format is typed UNSUPPORTED");
    if (err) {
        size_t elen = 0;
        const char *ejson = appname_error_json(err, &elen);
        check(ejson && strstr(ejson, "error.codec.unrecognized") != NULL,
              "reject carries the fixed section-8.1 taxonomy code");
        appname_error_free(err);
        err = NULL;
    } else {
        check(0, "export reject populates the error out-param");
    }

    /* 6d. Recovery surface (ABI 1.2 block): bind, query, end, discard. */
    char log_path[1024];
    snprintf(log_path, sizeof log_path, "%s/smoke.autosave", scratch);
    st = appname_doc_autosave_begin(doc, log_path, &err);
    check(st == APPNAME_OK, "appname_doc_autosave_begin");
    if (st != APPNAME_OK) { print_error("autosave_begin", err); appname_error_free(err); err = NULL; }
    AppnameString *query = NULL;
    st = appname_autosave_query(log_path, &query, &err);
    check(st == APPNAME_OK && query != NULL, "appname_autosave_query");
    if (query) {
        const char *qjson = appname_string_data(query, NULL);
        printf("autosave query: %s\n", qjson ? qjson : "(null)");
        check(qjson && strstr(qjson, "\"commands\":0") != NULL,
              "fresh log reports nothing to replay");
        check(appname_string_free(query) == APPNAME_OK, "query report freed");
    }
    check(appname_doc_autosave_end(doc, NULL) == APPNAME_OK, "appname_doc_autosave_end");
    check(appname_autosave_discard(log_path, NULL) == APPNAME_OK, "appname_autosave_discard");
    check(appname_autosave_discard(log_path, NULL) == APPNAME_OK,
          "double discard is an OK no-op (idempotent)");

    /* 6e. Action-invoke door (ABI 1.3 block): a two-step action replays
       with a report; a malformed action is a typed schema reject. */
    const char *action =
        "{\"schema\":\"appname.action.v1\",\"name\":\"smoke action\","
        "\"steps\":["
        "{\"command\":{\"kind\":\"layer.add\",\"params\":{\"kind\":\"raster\","
        "\"name\":\"ActionLayer\","
        "\"newLayerId\":\"7a13e000-0000-4000-8000-00000000c001\"}}},"
        "{\"stop\":{\"message\":\"smoke stop\",\"allowContinue\":true}}]}";
    AppnameString *action_report = NULL;
    st = appname_doc_apply_action(doc, (const uint8_t *)action, strlen(action),
                                  NULL, &action_report, &err);
    check(st == APPNAME_OK && action_report != NULL, "appname_doc_apply_action");
    if (st != APPNAME_OK) { print_error("apply_action", err); appname_error_free(err); err = NULL; }
    if (action_report) {
        const char *rjson = appname_string_data(action_report, NULL);
        printf("action report: %s\n", rjson ? rjson : "(null)");
        check(rjson && strstr(rjson, "\"status\":\"ok\"") != NULL,
              "action report status ok");
        check(rjson && strstr(rjson, "\"stop\"") != NULL,
              "stop step reported");
        check(appname_string_free(action_report) == APPNAME_OK, "action report freed");
        action_report = NULL;
    }
    st = appname_doc_apply_action(doc, (const uint8_t *)"{\"schema\":42}", 13,
                                  NULL, &action_report, &err);
    check(st == APPNAME_ERR_SCHEMA, "malformed action is typed SCHEMA");
    check(action_report == NULL, "reject leaves the report out-param untouched");
    if (err) {
        size_t elen = 0;
        const char *ejson = appname_error_json(err, &elen);
        check(ejson && strstr(ejson, "error.schema.wrong_schema") != NULL,
              "reject carries the schema taxonomy code");
        appname_error_free(err);
        err = NULL;
    } else {
        check(0, "action reject populates the error out-param");
    }

    /* 6f. Per-layer content readback (ABI 1.4 block) + info v3 paths. */
    {
        AppnameRect lrect = { 0, 0, 8, 8 };
        AppnameImageBuffer *lbuf = NULL;
        st = appname_doc_read_layer(doc, layer_id, lrect, &lbuf, &err);
        check(st == APPNAME_OK && lbuf != NULL, "appname_doc_read_layer");
        if (st != APPNAME_OK) { print_error("read_layer", err); appname_error_free(err); err = NULL; }
        if (lbuf) {
            AppnameMapInfo lmap;
            memset(&lmap, 0, sizeof lmap);
            check(appname_buffer_map(lbuf, &lmap) == APPNAME_OK && lmap.data != NULL,
                  "read_layer buffer maps");
            check(lmap.width == 8 && lmap.height == 8 && lmap.stride == 8 * 4
                      && lmap.pixel_format == 0 && lmap.color_model == 3
                      && lmap.premultiplied == 0,
                  "read_layer rides the read_region delivery convention");
            check(appname_buffer_unmap(lbuf) == APPNAME_OK, "read_layer unmap");
            check(appname_buffer_free(lbuf) == APPNAME_OK, "read_layer freed");
        }
        /* Unknown layer id: typed bad_target, never a crash. */
        lbuf = NULL;
        st = appname_doc_read_layer(doc, "00000000-dead-4000-8000-000000000000",
                                    lrect, &lbuf, &err);
        check(st == APPNAME_ERR_INVALID_ARG && lbuf == NULL,
              "read_layer of an unknown layer is typed INVALID_ARG");
        if (err) {
            size_t elen = 0;
            const char *ejson = appname_error_json(err, &elen);
            check(ejson && strstr(ejson, "error.command.bad_target") != NULL,
                  "unknown-layer payload is error.command.bad_target");
            appname_error_free(err);
            err = NULL;
        } else {
            check(0, "read_layer reject populates the error out-param");
        }
        /* info v3: the paths array is always present (empty here). */
        AppnameString *info3 = NULL;
        st = appname_doc_info_json(doc, &info3, &err);
        check(st == APPNAME_OK && info3 != NULL, "info for the v3 check");
        if (info3) {
            const char *i3 = appname_string_data(info3, NULL);
            check(i3 && strstr(i3, "\"paths\":[]") != NULL,
                  "info v3 carries the paths array (empty document)");
            check(appname_string_free(info3) == APPNAME_OK, "v3 info freed");
        }
    }

    /* 7. Close; misuse afterwards is TYPED, never undefined behavior. */
    check(appname_doc_close(doc) == APPNAME_OK, "appname_doc_close");
    check(appname_doc_close(doc) == APPNAME_ERR_INVALID_ARG, "double-close is typed");
    st = appname_doc_apply_command(doc, (const uint8_t *)envelope, strlen(envelope), NULL, &err);
    check(st == APPNAME_ERR_INVALID_ARG, "use-after-close is typed");
    if (err) {
        size_t elen = 0;
        const char *ejson = appname_error_json(err, &elen);
        check(ejson && strstr(ejson, "error.api.invalid_arg") != NULL,
              "use-after-close payload is error.api.invalid_arg");
        check(appname_error_free(err) == APPNAME_OK, "use-after-close error freed");
        err = NULL;
    } else {
        check(0, "use-after-close populates the error out-param");
    }

    /* 8. NULL tolerance of the free family (§9.3). */
    check(appname_doc_close(NULL) == APPNAME_OK, "close(NULL) is a no-op");
    check(appname_string_free(NULL) == APPNAME_OK, "string_free(NULL) is a no-op");
    check(appname_error_free(NULL) == APPNAME_OK, "error_free(NULL) is a no-op");
    check(appname_buffer_free(NULL) == APPNAME_OK, "buffer_free(NULL) is a no-op");
    check(appname_string_data(NULL, NULL) == NULL, "string_data(NULL) returns NULL");

    if (failures) {
        printf("cabi smoke: FAIL (%d)\n", failures);
        return 1;
    }
    printf("cabi smoke: PASS\n");
    return 0;
}
