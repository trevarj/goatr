//! JNI bridge between dev.herdr.terminal.GhosttyVt and the terminal session
//! (session.zig: ghostty's `ghostty-vt` Zig module, and the snapshot Kotlin
//! reads; its layout is documented there).
//!
//! One native handle owns a terminal, its VT stream, and a render state. All
//! calls come from the Android main thread, so nothing is locked.

const std = @import("std");
const c = @import("c");
const vt = @import("ghostty-vt");
const grid = @import("session.zig");
const Session = grid.Session;

const alloc = std.heap.c_allocator;
const log_tag = "herdr-terminal";

fn fromHandle(handle: c.jlong) ?*Session {
    if (handle == 0) return null;
    return @ptrFromInt(@as(usize, @intCast(handle)));
}

fn logWarn(comptime fmt: []const u8, args: anytype) void {
    var buf: [256]u8 = undefined;
    const msg = std.fmt.bufPrint(buf[0 .. buf.len - 1], fmt, args) catch return;
    buf[msg.len] = 0;
    _ = c.__android_log_write(c.ANDROID_LOG_WARN, log_tag, &buf);
}

// ---- JNI helpers -----------------------------------------------------------

const Env = [*c]c.JNIEnv;

fn jni(env: Env) *const c.struct_JNINativeInterface {
    return env.*;
}

/// The array's bytes, copied out (JNI may pin or copy either way). Free with alloc.
fn bytesOf(env: Env, array: c.jbyteArray) ?[]u8 {
    if (array == null) return null;
    const len: usize = @intCast(jni(env).GetArrayLength.?(env, array));
    const out = alloc.alloc(u8, len) catch return null;
    if (len > 0) jni(env).GetByteArrayRegion.?(env, array, 0, @intCast(len), @ptrCast(out.ptr));
    return out;
}

fn newBytes(env: Env, data: []const u8) c.jbyteArray {
    const out = jni(env).NewByteArray.?(env, @intCast(data.len));
    if (out == null) return null;
    if (data.len > 0) jni(env).SetByteArrayRegion.?(env, out, 0, @intCast(data.len), @ptrCast(data.ptr));
    return out;
}

/// Null for no bytes, which Kotlin reads as "nothing to send".
fn newBytesOrNull(env: Env, data: []const u8) c.jbyteArray {
    return if (data.len == 0) null else newBytes(env, data);
}

/// Parses a color in ghostty config syntax (hex with or without '#', X11
/// names, rgb:/rgbi: forms). Null on a null or invalid string.
fn colorOf(env: Env, str: c.jstring) ?vt.color.RGB {
    if (str == null) return null;
    const utf = jni(env).GetStringUTFChars.?(env, str, null);
    if (utf == null) return null;
    defer jni(env).ReleaseStringUTFChars.?(env, str, utf);
    const text = std.mem.span(utf);
    return vt.color.RGB.parse(text) catch {
        logWarn("ignoring invalid theme color: {s}", .{text});
        return null;
    };
}

// ---- Keys --------------------------------------------------------------------

fn keyOffset(base: vt.input.Key, offset: i32) vt.input.Key {
    return @fromBackingInt(@intCast(@backingInt(base) + offset));
}

fn mapKey(code: i32) vt.input.Key {
    if (code >= c.AKEYCODE_A and code <= c.AKEYCODE_Z) return keyOffset(.key_a, code - c.AKEYCODE_A);
    if (code >= c.AKEYCODE_0 and code <= c.AKEYCODE_9) return keyOffset(.digit_0, code - c.AKEYCODE_0);
    if (code >= c.AKEYCODE_NUMPAD_0 and code <= c.AKEYCODE_NUMPAD_9) return keyOffset(.numpad_0, code - c.AKEYCODE_NUMPAD_0);
    if (code >= c.AKEYCODE_F1 and code <= c.AKEYCODE_F12) return keyOffset(.f1, code - c.AKEYCODE_F1);
    return switch (code) {
        c.AKEYCODE_GRAVE => .backquote,
        c.AKEYCODE_BACKSLASH => .backslash,
        c.AKEYCODE_LEFT_BRACKET => .bracket_left,
        c.AKEYCODE_RIGHT_BRACKET => .bracket_right,
        c.AKEYCODE_COMMA => .comma,
        c.AKEYCODE_EQUALS => .equal,
        c.AKEYCODE_MINUS => .minus,
        c.AKEYCODE_PERIOD => .period,
        c.AKEYCODE_APOSTROPHE => .quote,
        c.AKEYCODE_SEMICOLON => .semicolon,
        c.AKEYCODE_SLASH => .slash,
        c.AKEYCODE_ALT_LEFT => .alt_left,
        c.AKEYCODE_ALT_RIGHT => .alt_right,
        c.AKEYCODE_DEL => .backspace,
        c.AKEYCODE_CAPS_LOCK => .caps_lock,
        c.AKEYCODE_CTRL_LEFT => .control_left,
        c.AKEYCODE_CTRL_RIGHT => .control_right,
        c.AKEYCODE_ENTER => .enter,
        c.AKEYCODE_META_LEFT => .meta_left,
        c.AKEYCODE_META_RIGHT => .meta_right,
        c.AKEYCODE_SHIFT_LEFT => .shift_left,
        c.AKEYCODE_SHIFT_RIGHT => .shift_right,
        c.AKEYCODE_SPACE => .space,
        c.AKEYCODE_TAB => .tab,
        c.AKEYCODE_FORWARD_DEL => .delete,
        c.AKEYCODE_MOVE_END => .end,
        c.AKEYCODE_MOVE_HOME => .home,
        c.AKEYCODE_INSERT => .insert,
        c.AKEYCODE_PAGE_DOWN => .page_down,
        c.AKEYCODE_PAGE_UP => .page_up,
        c.AKEYCODE_DPAD_DOWN => .arrow_down,
        c.AKEYCODE_DPAD_LEFT => .arrow_left,
        c.AKEYCODE_DPAD_RIGHT => .arrow_right,
        c.AKEYCODE_DPAD_UP => .arrow_up,
        c.AKEYCODE_NUM_LOCK => .num_lock,
        c.AKEYCODE_NUMPAD_ADD => .numpad_add,
        c.AKEYCODE_NUMPAD_COMMA => .numpad_comma,
        c.AKEYCODE_NUMPAD_DOT => .numpad_decimal,
        c.AKEYCODE_NUMPAD_DIVIDE => .numpad_divide,
        c.AKEYCODE_NUMPAD_ENTER => .numpad_enter,
        c.AKEYCODE_NUMPAD_EQUALS => .numpad_equal,
        c.AKEYCODE_NUMPAD_MULTIPLY => .numpad_multiply,
        c.AKEYCODE_NUMPAD_SUBTRACT => .numpad_subtract,
        c.AKEYCODE_ESCAPE => .escape,
        else => .unidentified,
    };
}

fn mapMods(meta: i32) vt.input.KeyMods {
    const on = struct {
        fn f(m: i32, bit: c_int) bool {
            return m & bit != 0;
        }
    }.f;
    var mods: vt.input.KeyMods = .{
        .shift = on(meta, c.AMETA_SHIFT_ON),
        .ctrl = on(meta, c.AMETA_CTRL_ON),
        .alt = on(meta, c.AMETA_ALT_ON),
        .super = on(meta, c.AMETA_META_ON),
        .caps_lock = on(meta, c.AMETA_CAPS_LOCK_ON),
        .num_lock = on(meta, c.AMETA_NUM_LOCK_ON),
    };
    if (on(meta, c.AMETA_SHIFT_RIGHT_ON)) mods.sides.shift = .right;
    if (on(meta, c.AMETA_CTRL_RIGHT_ON)) mods.sides.ctrl = .right;
    if (on(meta, c.AMETA_ALT_RIGHT_ON)) mods.sides.alt = .right;
    if (on(meta, c.AMETA_META_RIGHT_ON)) mods.sides.super = .right;
    return mods;
}

// ---- Natives (registered in JNI_OnLoad below) ---------------------------------

fn create(
    _: Env,
    _: c.jobject,
    cols: c.jint,
    rows: c.jint,
    max_scrollback: c.jlong,
) callconv(.c) c.jlong {
    const session = grid.create(cols, rows, max_scrollback) catch |err| {
        logWarn("failed to create terminal session: {s}", .{@errorName(err)});
        return 0;
    };
    return @intCast(@intFromPtr(session));
}

fn free(_: Env, _: c.jobject, handle: c.jlong) callconv(.c) void {
    grid.destroy(fromHandle(handle) orelse return);
}

fn write(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    data: c.jbyteArray,
) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    if (data == null) return null;
    session.pty_out.clearRetainingCapacity();
    const len = jni(env).GetArrayLength.?(env, data);
    if (len > 0) {
        const bytes = jni(env).GetByteArrayElements.?(env, data, null);
        if (bytes == null) return null;
        const slice: [*]const u8 = @ptrCast(bytes);
        session.stream.nextSlice(slice[0..@intCast(len)]);
        jni(env).ReleaseByteArrayElements.?(env, data, bytes, c.JNI_ABORT);
    }
    return newBytesOrNull(env, session.pty_out.items);
}

fn resize(
    _: Env,
    _: c.jobject,
    handle: c.jlong,
    cols: c.jint,
    rows: c.jint,
    cell_width_px: c.jint,
    cell_height_px: c.jint,
) callconv(.c) void {
    const session = fromHandle(handle) orelse return;
    if (cols <= 0 or rows <= 0) return;
    session.stream.handler.resize(.{
        .cols = @intCast(cols),
        .rows = @intCast(rows),
        .cell_size_px = .{
            .width = @intCast(@max(cell_width_px, 0)),
            .height = @intCast(@max(cell_height_px, 0)),
        },
    }) catch return;
    // The renderer discards its bitmap on every resize; a same-grid resize
    // (font-size change landing on identical cols/rows) would otherwise leave
    // no dirty rows to repaint it with.
    session.dirtyAll();
}

// Sets the terminal's default colors (theme; see Session.setTheme). Invalid
// color strings are logged and skipped, as ghostty does for bad config
// values. The palette overrides entries by index on top of ghostty's default
// 256-color palette.
fn setTheme(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    foreground: c.jstring,
    background: c.jstring,
    cursor: c.jstring,
    palette: c.jobjectArray,
) callconv(.c) void {
    const session = fromHandle(handle) orelse return;
    var table: vt.color.Palette = vt.color.default;
    if (palette != null) {
        const count: usize = @min(@as(usize, @intCast(jni(env).GetArrayLength.?(env, palette))), 256);
        for (0..count) |i| {
            const entry: c.jstring = @ptrCast(jni(env).GetObjectArrayElement.?(env, palette, @intCast(i)));
            if (colorOf(env, entry)) |rgb| table[i] = rgb;
            if (entry != null) jni(env).DeleteLocalRef.?(env, entry);
        }
    }
    session.setTheme(colorOf(env, foreground), colorOf(env, background), colorOf(env, cursor), table);
}

// Parses a color in ghostty config syntax to ARGB, or 0 when invalid (real
// colors always carry 0xFF alpha, so 0 is never a valid result).
fn parseColor(env: Env, _: c.jobject, color: c.jstring) callconv(.c) c.jint {
    return if (colorOf(env, color)) |rgb| grid.argb(rgb) else 0;
}

fn scroll(_: Env, _: c.jobject, handle: c.jlong, delta_rows: c.jint) callconv(.c) void {
    const session = fromHandle(handle) orelse return;
    session.term.scrollViewport(.{ .delta = delta_rows });
}

fn scrollToBottom(_: Env, _: c.jobject, handle: c.jlong) callconv(.c) void {
    const session = fromHandle(handle) orelse return;
    session.term.scrollViewport(.bottom);
}

fn snapshot(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    buffer: c.jobject,
) callconv(.c) c.jint {
    const session = fromHandle(handle) orelse return -1;
    if (buffer == null) return -1;
    const base_ptr: ?[*]u8 = @ptrCast(jni(env).GetDirectBufferAddress.?(env, buffer));
    const base = base_ptr orelse return -1;
    const cap: usize = @intCast(jni(env).GetDirectBufferCapacity.?(env, buffer));
    if (cap < grid.header_bytes) return -1;
    return grid.fillSnapshot(session, base[0..cap]) catch -1;
}

fn selectWord(
    _: Env,
    _: c.jobject,
    handle: c.jlong,
    col: c.jint,
    row: c.jint,
) callconv(.c) c.jboolean {
    const session = fromHandle(handle) orelse return c.JNI_FALSE;
    const pin = session.viewportPin(col, row) orelse return c.JNI_FALSE;
    const sel = session.term.screens.active.selectWord(pin, &grid.word_boundaries) orelse return c.JNI_FALSE;
    return @intFromBool(session.select(sel));
}

fn selectAll(_: Env, _: c.jobject, handle: c.jlong) callconv(.c) c.jboolean {
    const session = fromHandle(handle) orelse return c.JNI_FALSE;
    const sel = session.term.screens.active.selectAll() orelse return c.JNI_FALSE;
    return @intFromBool(session.select(sel));
}

fn setSelection(
    _: Env,
    _: c.jobject,
    handle: c.jlong,
    anchor_col: c.jint,
    anchor_row: c.jint,
    col: c.jint,
    row: c.jint,
) callconv(.c) c.jboolean {
    const session = fromHandle(handle) orelse return c.JNI_FALSE;
    const start = session.viewportPin(anchor_col, anchor_row) orelse return c.JNI_FALSE;
    const end = session.viewportPin(col, row) orelse return c.JNI_FALSE;
    return @intFromBool(session.select(.init(start, end, false)));
}

fn clearSelection(_: Env, _: c.jobject, handle: c.jlong) callconv(.c) void {
    const session = fromHandle(handle) orelse return;
    session.term.screens.active.clearSelection();
    session.reselect();
}

// The selection as UTF-8 (plain, unwrapped, trimmed: ghostty's copy
// semantics), or null when there's none.
fn selectionText(env: Env, _: c.jobject, handle: c.jlong) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    const sel = session.term.screens.active.selection orelse return null;
    var formatter: vt.formatter.TerminalFormatter = .init(&session.term, .{
        .emit = .plain,
        .unwrap = true,
        .trim = true,
    });
    formatter.content = .{ .selection = sel };
    var out: std.Io.Writer.Allocating = .init(alloc);
    defer out.deinit();
    formatter.format(&out.writer) catch return null;
    return newBytes(env, out.written());
}

/// The link at a viewport cell (see Session's linkAt), or null.
fn linkAt(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    col: c.jint,
    row: c.jint,
) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    const link = (grid.linkAt(session, col, row) catch return null) orelse return null;
    defer alloc.free(link.uri);
    return newBytes(env, link.uri);
}

/// The cells the link at a viewport cell covers, as [first, last] in
/// row-major viewport cells (y * cols + x), or null when there's none.
fn linkSpan(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    col: c.jint,
    row: c.jint,
) callconv(.c) c.jintArray {
    const session = fromHandle(handle) orelse return null;
    const link = (grid.linkAt(session, col, row) catch return null) orelse return null;
    defer alloc.free(link.uri);
    const cols = session.term.screens.active.pages.cols;
    const span = [2]c.jint{
        @intCast(link.first.y * cols + link.first.x),
        @intCast(link.last.y * cols + link.last.x),
    };
    const out = jni(env).NewIntArray.?(env, 2);
    if (out == null) return null;
    jni(env).SetIntArrayRegion.?(env, out, 0, 2, &span);
    return out;
}

/// Text a program copied (OSC 52) since the last call, or null.
fn takeClipboard(env: Env, _: c.jobject, handle: c.jlong) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    if (!session.clipboard_pending) return null;
    session.clipboard_pending = false;
    return newBytes(env, session.clipboard.items);
}

/// Whether programs may copy to the clipboard; refused writes are answered
/// as denied.
fn setClipboardWrite(_: Env, _: c.jobject, handle: c.jlong, allowed: c.jboolean) callconv(.c) void {
    const session = fromHandle(handle) orelse return;
    session.clipboard_allowed = allowed != c.JNI_FALSE;
}

fn isPasteSafe(env: Env, _: c.jobject, data: c.jbyteArray) callconv(.c) c.jboolean {
    const bytes = bytesOf(env, data) orelse return c.JNI_TRUE;
    defer alloc.free(bytes);
    return @intFromBool(vt.input.isSafePaste(bytes));
}

// Clipboard text for the PTY: unsafe control bytes stripped, and wrapped in
// bracketed-paste sequences when the program has mode 2004 set.
fn encodePaste(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    data: c.jbyteArray,
) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    const bytes = bytesOf(env, data) orelse return null;
    defer alloc.free(bytes);
    const parts = vt.input.encodePaste(bytes, .{ .bracketed = session.modeOn(.bracketed_paste) });
    const joined = std.mem.concat(alloc, u8, &parts) catch return null;
    defer alloc.free(joined);
    return newBytes(env, joined);
}

fn encodeKey(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    key_code: c.jint,
    action: c.jint,
    meta_state: c.jint,
    consumed_meta_state: c.jint,
    unshifted_codepoint: c.jint,
    utf8: c.jbyteArray,
) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    const text = bytesOf(env, utf8);
    defer if (text) |t| alloc.free(t);

    // Terminal modes (DECCKM, Kitty flags, ...) change as programs run; read
    // them before every encode so arrows and keypad follow the active mode.
    const opts: vt.input.KeyEncodeOptions = .fromTerminal(&session.term);
    const event: vt.input.KeyEvent = .{
        .action = switch (action) {
            0 => .release,
            2 => .repeat,
            else => .press,
        },
        .key = mapKey(key_code),
        .mods = mapMods(meta_state),
        // What the text used up: with kitty flags on, Shift+; is ':', not a shifted ';'.
        .consumed_mods = mapMods(consumed_meta_state),
        .utf8 = text orelse "",
        .unshifted_codepoint = std.math.cast(u21, unshifted_codepoint) orelse 0,
    };
    var out: std.Io.Writer.Allocating = .init(alloc);
    defer out.deinit();
    vt.input.encodeKey(&out.writer, event, opts) catch return null;
    return newBytesOrNull(env, out.written());
}

// What a vertical swipe means to the program in front: 1 when it tracks the
// mouse (send wheel events), 2 on the alternate screen without tracking (send
// arrow keys, like xterm's alternate scroll and Termux), 0 otherwise (scroll
// this view's scrollback).
fn scrollTarget(_: Env, _: c.jobject, handle: c.jlong) callconv(.c) c.jint {
    const session = fromHandle(handle) orelse return 0;
    if (session.modeOn(.mouse_event_x10) or session.modeOn(.mouse_event_normal) or
        session.modeOn(.mouse_event_button) or session.modeOn(.mouse_event_any)) return 1;
    if (session.modeOn(.alt_screen_save_cursor_clear_enter) or session.modeOn(.alt_screen) or
        session.modeOn(.alt_screen_legacy)) return 2;
    return 0;
}

// `count` wheel notches (up or down) at a pixel position, encoded for the
// program's mouse mode and format; null when it encodes to nothing.
fn encodeWheel(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    up: c.jboolean,
    count: c.jint,
    x: c.jfloat,
    y: c.jfloat,
    screen_width: c.jint,
    screen_height: c.jint,
    cell_width: c.jint,
    cell_height: c.jint,
) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    if (count <= 0 or cell_width <= 0 or cell_height <= 0) return null;
    const t = &session.term;
    const opts: vt.input.MouseEncodeOptions = .{
        .event = t.flags.mouse_event,
        .format = t.flags.mouse_format,
        .size = .{
            .screen = .{ .width = @intCast(@max(screen_width, 0)), .height = @intCast(@max(screen_height, 0)) },
            .cell = .{ .width = @intCast(cell_width), .height = @intCast(cell_height) },
            .padding = .{},
        },
    };
    const event: vt.input.MouseEncodeEvent = .{
        .action = .press,
        .button = if (up != 0) .four else .five,
        .pos = .{ .x = x, .y = y },
    };
    var out: std.Io.Writer.Allocating = .init(alloc);
    defer out.deinit();
    for (0..@intCast(count)) |_| vt.input.encodeMouse(&out.writer, event, opts) catch break;
    return newBytesOrNull(env, out.written());
}

// Every row again in the next snapshot: a view shown again has lost its drawn rows.
fn redraw(_: Env, _: c.jobject, handle: c.jlong) callconv(.c) void {
    const session = fromHandle(handle) orelse return;
    session.dirtyAll();
}

// A pointer's press (0), release (1) or motion (2) of `button` (0 none,
// 1 left, 2 right, 3 middle) at a pixel position, encoded for the program's
// mouse mode and format; null when it encodes to nothing (it doesn't track
// the mouse, or motion within the same cell).
fn encodeMouse(
    env: Env,
    _: c.jobject,
    handle: c.jlong,
    action: c.jint,
    button: c.jint,
    meta_state: c.jint,
    x: c.jfloat,
    y: c.jfloat,
    screen_width: c.jint,
    screen_height: c.jint,
    cell_width: c.jint,
    cell_height: c.jint,
    any_pressed: c.jboolean,
) callconv(.c) c.jbyteArray {
    const session = fromHandle(handle) orelse return null;
    if (cell_width <= 0 or cell_height <= 0) return null;
    const t = &session.term;
    const opts: vt.input.MouseEncodeOptions = .{
        .event = t.flags.mouse_event,
        .format = t.flags.mouse_format,
        .size = .{
            .screen = .{ .width = @intCast(@max(screen_width, 0)), .height = @intCast(@max(screen_height, 0)) },
            .cell = .{ .width = @intCast(cell_width), .height = @intCast(cell_height) },
            .padding = .{},
        },
        .any_button_pressed = any_pressed != 0,
        .last_cell = &session.mouse_cell,
    };
    const event: vt.input.MouseEncodeEvent = .{
        .action = switch (action) {
            0 => .press,
            1 => .release,
            else => .motion,
        },
        .button = switch (button) {
            1 => .left,
            2 => .right,
            3 => .middle,
            else => null,
        },
        .mods = mapMods(meta_state),
        .pos = .{ .x = x, .y = y },
    };
    var out: std.Io.Writer.Allocating = .init(alloc);
    defer out.deinit();
    vt.input.encodeMouse(&out.writer, event, opts) catch return null;
    return newBytesOrNull(env, out.written());
}

// ---- Registration ------------------------------------------------------------

/// GhosttyVt's external funs, by name and JNI signature.
const methods = [_]c.JNINativeMethod{
    method("create", "(IIJ)J", &create),
    method("free", "(J)V", &free),
    method("write", "(J[B)[B", &write),
    method("resize", "(JIIII)V", &resize),
    method("setTheme", "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;)V", &setTheme),
    method("parseColor", "(Ljava/lang/String;)I", &parseColor),
    method("scroll", "(JI)V", &scroll),
    method("scrollToBottom", "(J)V", &scrollToBottom),
    method("scrollTarget", "(J)I", &scrollTarget),
    method("encodeWheel", "(JZIFFIIII)[B", &encodeWheel),
    method("encodeMouse", "(JIIIFFIIIIZ)[B", &encodeMouse),
    method("redraw", "(J)V", &redraw),
    method("snapshot", "(JLjava/nio/ByteBuffer;)I", &snapshot),
    method("encodeKey", "(JIIIII[B)[B", &encodeKey),
    method("selectWord", "(JII)Z", &selectWord),
    method("selectAll", "(J)Z", &selectAll),
    method("setSelection", "(JIIII)Z", &setSelection),
    method("clearSelection", "(J)V", &clearSelection),
    method("selectionText", "(J)[B", &selectionText),
    method("linkAt", "(JII)[B", &linkAt),
    method("linkSpan", "(JII)[I", &linkSpan),
    method("takeClipboard", "(J)[B", &takeClipboard),
    method("setClipboardWrite", "(JZ)V", &setClipboardWrite),
    method("isPasteSafe", "([B)Z", &isPasteSafe),
    method("encodePaste", "(J[B)[B", &encodePaste),
};

fn method(comptime name: [:0]const u8, comptime signature: [:0]const u8, func: *const anyopaque) c.JNINativeMethod {
    return .{ .name = @constCast(name.ptr), .signature = @constCast(signature.ptr), .fnPtr = @constCast(func) };
}

/// Hands Kotlin's GhosttyVt its natives when the library loads, so the
/// functions above keep short names instead of JNI's `Java_<package>_<class>_<method>`.
export fn JNI_OnLoad(vm: *c.JavaVM, _: ?*anyopaque) callconv(.c) c.jint {
    var env: Env = null;
    if (vm.*.*.GetEnv.?(vm, @ptrCast(&env), c.JNI_VERSION_1_6) != c.JNI_OK) return c.JNI_ERR;
    const class = jni(env).FindClass.?(env, "dev/herdr/terminal/GhosttyVt");
    if (class == null) return c.JNI_ERR;
    if (jni(env).RegisterNatives.?(env, class, &methods, methods.len) != c.JNI_OK) return c.JNI_ERR;
    return c.JNI_VERSION_1_6;
}
