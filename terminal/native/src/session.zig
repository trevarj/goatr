//! A terminal session: ghostty's terminal, its VT stream and a render state,
//! with what the view reads of it (the snapshot) and asks of it (selection,
//! links). No JNI here: jni.zig wraps it for Kotlin, and tests and benchmarks
//! use it directly on this machine.
//!
//! snapshot flattens the dirty viewport rows into a caller-provided
//! direct ByteBuffer (little-endian) so Kotlin can repaint without any
//! per-cell JNI calls. See GhosttyTerminalView.kt for the reader side.
//!
//! Buffer layout:
//!   header: 22 x i32
//!     [0] version (6)          [1] dirty kind (0 none / 1 partial / 2 full)
//!     [2] cols                 [3] rows
//!     [4] default bg (ARGB)    [5] default fg (ARGB)
//!     [6] cursor x or -1 (a wide character's first cell when on either)
//!     [7] cursor y or -1
//!     [8] cursor style         [9] cursor visible (0/1)
//!     [10] row record count    [11] cursor blinking (0/1)
//!     [12] cursor color (ARGB) or 0 when unconfigured
//!     [13] held (0/1): synchronized output; the rows are the frame captured
//!          when it began, and the caller should look again shortly
//!     [14] cursor wide (0/1): on a wide character, two cells wide
//!     [15] the cursor's cell's flags (as a cell's, below)
//!     [16] its text's length in UTF-16 units (0-4, the first of a longer one)
//!     [17..18] its text: 4 x u16 — a block cursor draws it over itself
//!     [19] scrollbar: rows in all  [20] the viewport's first  [21] its rows
//!   row record:
//!     i32 rowIndex, i32 selStartX or -1, i32 selEndX or -1, i32 textUnits,
//!     i32 movedFrom: -1, or the row whose drawing this one is, unchanged
//!       (the viewport moved): then nothing follows, cells or text
//!     cells[cols] x 20 bytes:
//!       i32 fg (ARGB), i32 bg (ARGB),
//!       u16 textOffset (UTF-16 units into row blob), u16 textLen (units),
//!       u16 flags, u16 underline style (0 none, 1 single, 2 double,
//!       3 curly, 4 dotted, 5 dashed), i32 underline color (ARGB) or 0
//!     u16 text[textUnits], zero-padded to a 4-byte boundary
//!
//! Rows left out are as the view drew them: unchanged, where they were. A
//! row the program rewrote as it was isn't sent again; one that only moved
//! (scrolled) comes as where it was.

const std = @import("std");
const vt = @import("ghostty-vt");

const alloc = std.heap.c_allocator;

// Cell flag bits shared with GhosttyTerminalView.kt.
const flag_bold: u16 = 1 << 0;
const flag_italic: u16 = 1 << 1;
const flag_faint: u16 = 1 << 2;
const flag_underline: u16 = 1 << 3;
const flag_strikethrough: u16 = 1 << 4;
const flag_inverse: u16 = 1 << 5;
const flag_invisible: u16 = 1 << 6;
const flag_wide: u16 = 1 << 7;
const flag_spacer: u16 = 1 << 8;
const flag_fg_default: u16 = 1 << 9;
const flag_bg_none: u16 = 1 << 10;
const flag_overline: u16 = 1 << 11;

pub const header_bytes = 22 * 4;
// Colors when the theme gives none.
const default_background: vt.color.RGB = .{ .r = 0x00, .g = 0x00, .b = 0x00 };
const default_foreground: vt.color.RGB = .{ .r = 0xFF, .g = 0xFF, .b = 0xFF };
const row_header_bytes = 5 * 4;
const cell_record_bytes = 20;
// How long a synchronized-output hold may last before drawing resumes anyway.
const max_hold_ms = 1000;
// The most a program may copy to the clipboard at once (OSC 52).
const max_clipboard_bytes = 1 << 20;
// Cap per-cell grapheme text; anything longer is truncated (degenerate input).
const max_cell_text_units = 32;

// Default boundaries for word selection, as ghostty's own
// (terminal/selection_codepoints.zig, which the module doesn't export).
pub const word_boundaries = [_]u21{ 0, ' ', '\t', '\'', '"', '│', '`', '|', ':', ';', ',', '(', ')', '[', ']', '{', '}', '<', '>', '$' };

/// A row as the view drew it: the row of the terminal it showed (ghostty's
/// row id) and a hash of how it looked.
const Shown = struct {
    serial: u64,
    y: u32,
    hash: u64,
};

pub const Session = struct {
    io: vt.TinyIo = .init,
    term: vt.Terminal,
    stream: vt.TerminalStream,
    render: vt.RenderState = .empty,
    // Query responses emitted by the terminal while writing; handed back to
    // the caller after each write, for the PTY.
    pty_out: std.ArrayList(u8) = .empty,
    // When a synchronized-output hold began (monotonic ms), while it lasts.
    held_since: ?i64 = null,
    // The default colors the last snapshot gave, which the view drew its rows in.
    shown_colors: ?[2]vt.color.RGB = null,
    // Where the cursor was (x, y) when the view last drew its rows.
    shown_cursor: ?[2]usize = null,
    // What each of the view's rows was drawn from (empty: nothing, every row
    // is sent), and room for the next snapshot's.
    shown_rows: std.ArrayList(Shown) = .empty,
    next_rows: std.ArrayList(Shown) = .empty,
    // Text a program copied (OSC 52), until takeClipboard collects it.
    clipboard: std.ArrayList(u8) = .empty,
    clipboard_pending: bool = false,
    clipboard_allowed: bool = true,
    // The cell last reported for motion, so a pointer moving within a cell sends nothing.
    mouse_cell: ?vt.Coordinate = null,

    fn fromHandler(handler: *vt.TerminalStream.Handler) *Session {
        const stream: *vt.TerminalStream = @fieldParentPtr("handler", handler);
        return @alignCast(@fieldParentPtr("stream", stream));
    }

    fn writePty(handler: *vt.TerminalStream.Handler, data: []const u8) void {
        const self = fromHandler(handler);
        self.pty_out.appendSlice(alloc, data) catch {};
    }

    // Synchronized output (mode 2026): capture the finished frame the program
    // wants on screen, then draw nothing newer until the hold ends.
    fn renderHold(handler: *vt.TerminalStream.Handler, held: bool) void {
        const self = fromHandler(handler);
        if (held) {
            self.render.update(alloc, &self.term) catch {};
            self.held_since = nowMs();
        } else {
            self.held_since = null;
        }
    }

    // A program copying text (OSC 52, kitty's OSC 5522), kept for
    // takeClipboard. Android has one clipboard: the standard one.
    fn clipboardWrite(handler: *vt.TerminalStream.Handler, request: vt.clipboard.Write) void {
        const self = fromHandler(handler);
        if (!self.clipboard_allowed) return request.reply(.denied);
        if (request.location != .standard) return request.reply(.unsupported);
        for (request.contents) |content| {
            if (!vt.clipboard.isTextMime(content.mime)) continue;
            if (content.data.len > max_clipboard_bytes) return request.reply(.invalid_data);
            self.clipboard.clearRetainingCapacity();
            self.clipboard.appendSlice(alloc, content.data) catch return request.reply(.io_error);
            self.clipboard_pending = true;
            return request.reply(.{ .success = .{} });
        }
        request.reply(.unsupported);
    }

    // Answered like the C API does: with the default attributes.
    fn deviceAttributes(_: *vt.TerminalStream.Handler) vt.device_attributes.Attributes {
        return .{};
    }

    /// Every row again in the next snapshot: the view lost its drawings (a
    /// resize, a theme, a view shown again).
    pub fn dirtyAll(self: *Session) void {
        self.render.dirty = .full;
        self.shown_rows.clearRetainingCapacity();
    }

    /// The selection changed. It doesn't reliably mark rows dirty, so every
    /// row is looked at again; those whose selection is as it was (most, as
    /// a handle is dragged) aren't sent.
    pub fn reselect(self: *Session) void {
        self.render.dirty = .full;
    }

    pub fn select(self: *Session, sel: vt.Selection) bool {
        self.term.screens.active.select(sel) catch return false;
        self.reselect();
        return true;
    }

    /// The default colors (a theme): null foreground and background are
    /// the built-in white on black, a null cursor follows the text. OSC 4
    /// overrides set by the running program are kept by the terminal.
    pub fn setTheme(self: *Session, fg: ?vt.color.RGB, bg: ?vt.color.RGB, cursor: ?vt.color.RGB, palette: vt.color.Palette) void {
        const t = &self.term;
        // Never none: with either unset, the render state keeps whatever
        // colors it had and the view draws in a theme that's gone.
        t.colors.foreground.default = fg orelse default_foreground;
        t.colors.background.default = bg orelse default_background;
        t.colors.cursor.default = cursor;
        t.colors.palette.changeDefault(t.gpa(), palette) catch {};
        t.flags.dirty.palette = true;
        self.dirtyAll();
    }

    pub fn viewportPin(self: *Session, col: i32, row: i32) ?vt.Pin {
        if (col < 0 or row < 0) return null;
        return self.term.screens.active.pages.pin(.{ .viewport = .{
            .x = @intCast(col),
            .y = @intCast(row),
        } });
    }

    pub fn modeOn(self: *Session, comptime mode: vt.Mode) bool {
        return self.term.modes.get(mode);
    }
};

/// A terminal of `cols` × `rows` keeping `max_scrollback` lines. Free with `destroy`.
pub fn create(cols: i32, rows: i32, max_scrollback: i64) !*Session {
    if (cols <= 0 or rows <= 0) return error.InvalidSize;
    const session = try alloc.create(Session);
    errdefer alloc.destroy(session);
    session.* = .{ .term = undefined, .stream = undefined };
    session.term = try .init(session.io.io(), alloc, .{
        .cols = @intCast(cols),
        .rows = @intCast(rows),
    });
    const t = &session.term;
    // We don't install ghostty's shell integration, so don't assume prompts
    // can be redrawn on resize (shells can still opt in with OSC 133).
    t.flags.shell_redraws_prompt = .false;
    t.setScrollbackMaxLines(@intCast(max_scrollback));

    var handler = t.vtHandler();
    handler.effects.write_pty = &Session.writePty;
    handler.effects.device_attributes = &Session.deviceAttributes;
    handler.effects.render_hold = &Session.renderHold;
    handler.effects.clipboard_write = &Session.clipboardWrite;
    session.stream = .init(.{ .allocator = alloc, .handler = handler });

    t.colors.background.default = default_background;
    t.colors.foreground.default = default_foreground;
    t.flags.dirty.palette = true;
    // libghostty's default is a steady cursor; blink by default like the
    // terminal apps users come from. DECSCUSR still overrides.
    t.setDefaultCursorBlink(true);
    // On from the start (and after a reset), as in Ghostty and herdr-app:
    // VS16 makes ✍️ two cells wide, ZWJ sequences one character.
    t.modes.setDefault(.grapheme_cluster, true);
    t.modes.set(.grapheme_cluster, true);
    return session;
}

/// Frees a session from `create`.
pub fn destroy(session: *Session) void {
    session.render.deinit(alloc);
    session.stream.deinit();
    session.term.deinit(alloc);
    session.pty_out.deinit(alloc);
    session.clipboard.deinit(alloc);
    session.shown_rows.deinit(alloc);
    session.next_rows.deinit(alloc);
    alloc.destroy(session);
}

pub fn argb(rgb: vt.color.RGB) i32 {
    return @bitCast(0xFF000000 | (@as(u32, rgb.r) << 16) | (@as(u32, rgb.g) << 8) | rgb.b);
}

/// Appends one codepoint as UTF-16; returns the units written (0 if full).
fn appendUtf16(cp: u21, out: []u16, used: usize) usize {
    if (cp < 0x10000) {
        if (used + 1 > out.len) return 0;
        out[used] = @intCast(cp);
        return 1;
    }
    if (used + 2 > out.len) return 0;
    const v = cp - 0x10000;
    out[used] = @intCast(0xD800 + (v >> 10));
    out[used + 1] = @intCast(0xDC00 + (v & 0x3FF));
    return 2;
}

fn putI32(buf: []u8, at: usize, v: i32) void {
    std.mem.writeInt(i32, buf[at..][0..4], v, .little);
}

fn putU16(buf: []u8, at: usize, v: u16) void {
    std.mem.writeInt(u16, buf[at..][0..2], v, .little);
}

pub fn nowMs() i64 {
    var ts: std.c.timespec = undefined;
    _ = std.c.clock_gettime(.MONOTONIC, &ts);
    return @as(i64, ts.sec) * 1000 + @divTrunc(@as(i64, ts.nsec), 1_000_000);
}

/// A style's flags, as a cell record carries them.
fn styleFlags(style: vt.Style) u16 {
    var flags: u16 = 0;
    if (style.flags.bold) flags |= flag_bold;
    if (style.flags.italic) flags |= flag_italic;
    if (style.flags.faint) flags |= flag_faint;
    if (style.flags.underline != .none) flags |= flag_underline;
    if (style.flags.strikethrough) flags |= flag_strikethrough;
    if (style.flags.inverse) flags |= flag_inverse;
    if (style.flags.invisible) flags |= flag_invisible;
    if (style.flags.overline) flags |= flag_overline;
    return flags;
}

/// Writes the snapshot (see the layout above) into `buf`; returns the row records written.
pub fn fillSnapshot(session: *Session, buf: []u8) !i32 {
    const state = &session.render;
    // During a hold the render state keeps the frame captured when it began.
    // A program that never ends its hold doesn't freeze the screen.
    if (session.held_since) |since| {
        if (nowMs() - since > max_hold_ms) {
            session.term.modes.set(.synchronized_output, false);
            session.held_since = null;
        }
    }
    const held = session.held_since != null;
    if (!held) try state.update(alloc, &session.term);

    // New default colors (OSC 10/11, reverse video) mark no row, but every
    // row was drawn in the old ones: all of them again.
    const colors = [2]vt.color.RGB{ state.colors.background, state.colors.foreground };
    const recolored = if (session.shown_colors) |shown| !std.meta.eql(shown, colors) else false;
    session.shown_colors = colors;
    // The cursor's rows: the one it left and the one it's on, drawn again
    // with it (a ligature run ends at the cursor).
    const at: ?[2]usize = if (state.cursor.viewport) |vp| .{ vp.x, vp.y } else null;
    // Those rows are drawn again even when they look as they did.
    var cursor_rows: [2]?usize = .{ null, null };
    if (!std.meta.eql(at, session.shown_cursor)) {
        if (session.shown_cursor) |p| cursor_rows[0] = p[1];
        if (at) |p| cursor_rows[1] = p[1];
        const range = state.rowDataRange();
        const row_dirty = state.row_data.items(.dirty)[range.start..range.end];
        for ([_]?[2]usize{ session.shown_cursor, at }) |cursor_at| {
            const p = cursor_at orelse continue;
            if (p[1] < row_dirty.len) row_dirty[p[1]] = true;
        }
        if (state.dirty == .false) state.dirty = .partial;
        session.shown_cursor = at;
    }
    const dirty: @TypeOf(state.dirty) = if (recolored) .full else state.dirty;
    const cols: usize = state.cols;
    const cursor = state.cursor;
    const cursor_color: i32 = if (session.term.colors.cursor.get()) |rgb| argb(rgb) else 0;
    const visual: i32 = switch (cursor.visual_style) {
        .bar => 0,
        .block => 1,
        .underline => 2,
        .block_hollow => 3,
    };

    // On a wide character's right half, the cursor is on the character: its first cell, two wide.
    const wide = switch (cursor.cell.wide) {
        .wide, .spacer_tail => true,
        .narrow, .spacer_head => false,
    };
    const cursor_x: i32 = if (cursor.viewport) |vp| @as(i32, vp.x) - @intFromBool(vp.wide_tail) else -1;

    // The cell under the cursor, as it's drawn: what a block cursor draws again over itself.
    var glyph: [4]u16 = @splat(0);
    var glyph_units: usize = 0;
    var glyph_flags: u16 = 0;
    if (cursor.viewport) |vp| glyph: {
        const range = state.rowDataRange();
        const y = range.start + vp.y;
        if (y >= range.end) break :glyph;
        const data = state.row_data.items(.cells)[y].slice();
        const x: usize = @intCast(cursor_x);
        if (x >= data.len) break :glyph;
        const cell = data.items(.raw)[x];
        const style: vt.Style = if (cell.hasStyling()) data.items(.style)[x] else .{};
        glyph_flags = styleFlags(style);
        if (!cell.hasText() or style.flags.invisible) break :glyph;
        glyph_units = appendUtf16(cell.codepoint(), &glyph, 0);
    }

    // Where the viewport is in the scrollback, for the view's scrollbar.
    const bar = session.term.screens.active.pages.scrollbar();

    const header = [_]i32{
        6,
        @backingInt(dirty),
        @intCast(cols),
        @intCast(state.rows),
        argb(state.colors.background),
        argb(state.colors.foreground),
        cursor_x,
        if (cursor.viewport) |vp| @intCast(vp.y) else -1,
        if (cursor.viewport != null) visual else 1,
        @intFromBool(cursor.visible),
        0, // row record count, set below
        @intFromBool(cursor.blinking),
        cursor_color,
        @intFromBool(held),
        @intFromBool(cursor.viewport != null and wide),
        glyph_flags,
        @intCast(glyph_units),
        @bitCast(@as(u32, glyph[0]) | @as(u32, glyph[1]) << 16),
        @bitCast(@as(u32, glyph[2]) | @as(u32, glyph[3]) << 16),
        @intCast(@min(bar.total, std.math.maxInt(i32))),
        @intCast(@min(bar.offset, std.math.maxInt(i32))),
        @intCast(@min(bar.len, std.math.maxInt(i32))),
    };
    comptime std.debug.assert(header.len * 4 == header_bytes);
    for (header, 0..) |v, i| putI32(buf, i * 4, v);

    if (dirty == .false) return 0;

    const range = state.rowDataRange();
    const rows = state.row_data.slice();
    const cells_of = rows.items(.cells)[range.start..range.end];
    const selections = rows.items(.selection)[range.start..range.end];
    const row_dirty = rows.items(.dirty)[range.start..range.end];
    const palette = &state.colors.palette;

    var offset: usize = header_bytes;
    var row_count: i32 = 0;

    // What the view's rows show, to send only what changed: a row the program
    // rewrote as it was isn't sent, one that only moved is sent as where it was.
    const serials = rows.items(.serial)[range.start..range.end];
    const pins = rows.items(.pin)[range.start..range.end];
    const shown = session.shown_rows.items;
    const known = shown.len == cells_of.len;
    try session.next_rows.resize(alloc, cells_of.len);
    const next = session.next_rows.items;
    const look = lookHash(state);
    var shift: isize = 0;

    for (cells_of, 0..) |row_cells, y| {
        const was_dirty = row_dirty[y];
        row_dirty[y] = false;
        if (dirty != .full and !was_dirty) {
            next[y] = if (known) shown[y] else .{ .serial = 0, .y = 0, .hash = 0 };
            continue;
        }

        const sel = selections[y];
        next[y] = .{ .serial = serials[y], .y = @intCast(pins[y].y), .hash = rowHash(row_cells, sel, look) };
        const moved_from: ?usize = moved: {
            if (!known or y == cursor_rows[0] or y == cursor_rows[1]) break :moved null;
            // Rows scroll together: by as many as the row before, most likely.
            const guess = @as(isize, @intCast(y)) + shift;
            if (guess >= 0 and guess < shown.len and std.meta.eql(shown[@intCast(guess)], next[y])) break :moved @intCast(guess);
            for (shown, 0..) |row, from| {
                if (std.meta.eql(row, next[y])) break :moved from;
            }
            break :moved null;
        };
        if (moved_from) |from| shift = @as(isize, @intCast(from)) - @as(isize, @intCast(y));
        // As it was, where it was: nothing to send.
        if (moved_from == y) continue;

        if (offset + row_header_bytes > buf.len) return error.NoSpace;
        putI32(buf, offset, @intCast(y));
        putI32(buf, offset + 4, if (sel) |s| @intCast(s[0]) else -1);
        putI32(buf, offset + 8, if (sel) |s| @intCast(s[1]) else -1);
        if (moved_from) |from| {
            putI32(buf, offset + 12, 0);
            putI32(buf, offset + 16, @intCast(from));
            offset += row_header_bytes;
            row_count += 1;
            continue;
        }
        putI32(buf, offset + 16, -1);

        const cells_offset = offset + row_header_bytes;
        const text_offset = cells_offset + cols * cell_record_bytes;
        if (text_offset > buf.len) return error.NoSpace;

        const text_cap = (buf.len - text_offset) / 2;
        const text: []u16 = @as([*]u16, @ptrCast(@alignCast(buf.ptr + text_offset)))[0..text_cap];
        var text_units: usize = 0;

        const data = row_cells.slice();
        const raws = data.items(.raw);
        const graphemes = data.items(.grapheme);
        const styles = data.items(.style);

        var col: usize = 0;
        while (col < cols and col < raws.len) : (col += 1) {
            const cell = raws[col];
            const record = cells_offset + col * cell_record_bytes;
            var flags: u16 = 0;

            switch (cell.wide) {
                .wide => flags |= flag_wide,
                .spacer_tail, .spacer_head => flags |= flag_spacer,
                else => {},
            }

            // Style data is undefined for default-styled cells.
            const style: vt.Style = if (cell.hasStyling()) styles[col] else .{};
            flags |= styleFlags(style);

            const fg: ?vt.color.RGB = switch (style.fg_color.value()) {
                .none => null,
                .palette => |idx| palette[idx],
                .rgb => |rgb| rgb,
            };
            // A bg-only cell carries its color in the content.
            const bg = style.bg(&cell, palette);
            if (fg == null) flags |= flag_fg_default;
            if (bg == null) flags |= flag_bg_none;

            const start = text_units;
            var units: usize = 0;
            if (flags & flag_spacer == 0 and cell.hasText()) {
                const first = appendUtf16(cell.codepoint(), text, text_units);
                units += first;
                if (first > 0 and cell.hasGrapheme()) {
                    for (graphemes[col]) |cp| {
                        if (units >= max_cell_text_units) break;
                        const n = appendUtf16(cp, text, text_units + units);
                        if (n == 0) break;
                        units += n;
                    }
                }
            }
            text_units += units;

            putI32(buf, record, if (fg) |rgb| argb(rgb) else 0);
            putI32(buf, record + 4, if (bg) |rgb| argb(rgb) else 0);
            putU16(buf, record + 8, @intCast(start));
            putU16(buf, record + 10, @intCast(units));
            putU16(buf, record + 12, flags);
            putU16(buf, record + 14, @intFromEnum(style.flags.underline));
            const underline_color: ?vt.color.RGB = switch (style.underline_color.value()) {
                .none => null,
                .palette => |idx| palette[idx],
                .rgb => |rgb| rgb,
            };
            putI32(buf, record + 16, if (underline_color) |rgb| argb(rgb) else 0);
        }
        // Zero-fill any remaining cell records (defensive; cols should match).
        while (col < cols) : (col += 1) {
            const record = cells_offset + col * cell_record_bytes;
            @memset(buf[record..][0..cell_record_bytes], 0);
            putU16(buf, record + 12, flag_fg_default | flag_bg_none);
        }

        putI32(buf, offset + 12, @intCast(text_units));
        offset = text_offset + text_units * 2;
        if (offset % 4 != 0) {
            if (offset + 2 > buf.len) return error.NoSpace;
            putU16(buf, offset, 0);
            offset += 2;
        }
        row_count += 1;
    }

    std.mem.swap(std.ArrayList(Shown), &session.shown_rows, &session.next_rows);
    state.dirty = .false;
    putI32(buf, 40, row_count);
    return row_count;
}

/// A hash of the colors cells are drawn in: the palette and the defaults.
fn lookHash(state: *const vt.RenderState) u64 {
    var h = std.hash.Wyhash.init(0);
    h.update(std.mem.asBytes(&state.colors.palette));
    h.update(std.mem.asBytes(&state.colors.background));
    h.update(std.mem.asBytes(&state.colors.foreground));
    return h.final();
}

/// A hash of how a row looks: its cells (characters, styles), its selection
/// and the colors it's drawn in.
fn rowHash(row_cells: anytype, sel: ?[2]vt.size.CellCountInt, look: u64) u64 {
    var h = std.hash.Wyhash.init(look);
    const data = row_cells.slice();
    const raws = data.items(.raw);
    const graphemes = data.items(.grapheme);
    const styles = data.items(.style);
    h.update(std.mem.sliceAsBytes(raws));
    for (raws, 0..) |cell, i| {
        // Style data is undefined for default-styled cells.
        if (cell.hasStyling()) h.update(std.mem.asBytes(&styles[i]));
        if (cell.hasGrapheme()) h.update(std.mem.sliceAsBytes(graphemes[i]));
    }
    h.update(std.mem.asBytes(&sel));
    return h.final();
}

/// The link at a viewport cell and the cells it covers, row-major: its OSC 8
/// hyperlink, or else the http(s) URL written across the cell's line (soft
/// wraps included). `uri` is the caller's (`alloc`).
pub const Link = struct {
    uri: []u8,
    first: Cell,
    last: Cell,

    pub const Cell = struct { x: u32, y: u32 };
};

pub fn linkAt(session: *Session, col: i32, row: i32) !?Link {
    var pin = session.viewportPin(col, row) orelse return null;
    // A wide character's right half belongs to its left.
    if (pin.rowAndCell().cell.wide == .spacer_tail and pin.x > 0) pin.x -= 1;
    const screen = session.term.screens.active;
    const cols: u32 = screen.pages.cols;
    const rows: u32 = screen.pages.rows;

    if (hyperlinkUri(pin)) |uri| {
        // Its cells: the neighbors, either way, with the same link.
        const at: u32 = @intCast(row * @as(i32, @intCast(cols)) + col);
        const same = struct {
            fn f(s: *Session, cell: u32, c: u32, link: []const u8) bool {
                const p = s.viewportPin(@intCast(cell % c), @intCast(cell / c)) orelse return false;
                const other = hyperlinkUri(p) orelse return false;
                return std.mem.eql(u8, other, link);
            }
        }.f;
        var first = at;
        while (first > 0 and same(session, first - 1, cols, uri)) first -= 1;
        var last = at;
        while (last + 1 < cols * rows and same(session, last + 1, cols, uri)) last += 1;
        return .{
            .uri = try alloc.dupe(u8, uri),
            .first = .{ .x = first % cols, .y = first / cols },
            .last = .{ .x = last % cols, .y = last / cols },
        };
    }

    const sel = screen.selectLine(.{
        .pin = pin,
        .whitespace = null,
        .semantic_prompt_boundary = false,
    }) orelse return null;
    var line = try screen.selectionStringMap(alloc, .{ .sel = sel, .trim = false });
    defer line.deinit(alloc);
    for (0..line.map.count()) |i| {
        const here = line.map.get(i) orelse continue;
        if (here.node != pin.node or here.x != pin.x or here.y != pin.y) continue;
        const url = urlAround(line.string, i) orelse return null;
        const start = @intFromPtr(url.ptr) - @intFromPtr(line.string.ptr);
        const first = viewportCell(screen, line.map.get(start)) orelse return null;
        const last = viewportCell(screen, line.map.get(start + url.len - 1)) orelse return null;
        return .{ .uri = try alloc.dupe(u8, url), .first = first, .last = last };
    }
    return null;
}

fn viewportCell(screen: anytype, pin: ?vt.Pin) ?Link.Cell {
    const p = screen.pages.pointFromPin(.viewport, pin orelse return null) orelse return null;
    return .{ .x = p.viewport.x, .y = p.viewport.y };
}

pub fn hyperlinkUri(pin: vt.Pin) ?[]const u8 {
    const page = pin.node.page();
    const cell = page.getRowAndCell(pin.x, pin.y).cell;
    if (!cell.hyperlink) return null;
    const id = page.lookupHyperlink(cell) orelse return null;
    return page.hyperlink_set.get(page.memory, id).uri.slice(page.memory);
}

/// The http(s) URL in `text` that covers byte `at`, less the punctuation
/// that usually follows one in prose (a period, an unmatched parenthesis).
pub fn urlAround(text: []const u8, at: usize) ?[]const u8 {
    var start = at + 1;
    while (true) {
        if (start == 0) return null;
        start -= 1;
        const rest = text[start..];
        if (std.mem.startsWith(u8, rest, "https://") or std.mem.startsWith(u8, rest, "http://")) break;
        if (endsUrl(text[start])) return null;
    }
    var end = start;
    while (end < text.len and !endsUrl(text[end])) end += 1;
    while (end > start) {
        const url = text[start..end];
        const last = url[url.len - 1];
        const unmatched = (last == ')' and std.mem.count(u8, url, "(") < std.mem.count(u8, url, ")")) or
            (last == ']' and std.mem.count(u8, url, "[") < std.mem.count(u8, url, "]"));
        if (!unmatched and std.mem.indexOfScalar(u8, ".,;:!?", last) == null) break;
        end -= 1;
    }
    const url = text[start..end];
    if (at >= end or std.mem.endsWith(u8, url, "://")) return null;
    return url;
}

/// Bytes a URL doesn't run through: whitespace and controls, quotes and angle
/// brackets, and anything non-ASCII (a TUI's box drawing around it).
fn endsUrl(byte: u8) bool {
    return byte <= ' ' or byte >= 0x7F or std.mem.indexOfScalar(u8, "<>\"'`", byte) != null;
}


/// The bytes a snapshot of a `cols` × `rows` grid may take, as the view sizes its buffer.
pub fn bufferSize(cols: usize, rows: usize) usize {
    return header_bytes + rows * (row_header_bytes + cols * cell_record_bytes + cols * max_cell_text_units * 2 + 4);
}

/// A snapshot buffer for a `cols` × `rows` grid, as the view sizes it.
fn testBuffer(comptime cols: usize, comptime rows: usize) type {
    return [bufferSize(cols, rows)]u8;
}

/// The header's dirty kind and row record count, after a snapshot.
fn headerOf(buf: []const u8) struct { dirty: i32, rows: i32, bg: i32, fg: i32, cursor_x: i32, cursor_wide: i32 } {
    const at = struct {
        fn f(b: []const u8, i: usize) i32 {
            return std.mem.readInt(i32, b[i * 4 ..][0..4], .little);
        }
    }.f;
    return .{
        .dirty = at(buf, 1),
        .rows = at(buf, 10),
        .bg = at(buf, 4),
        .fg = at(buf, 5),
        .cursor_x = at(buf, 6),
        .cursor_wide = at(buf, 14),
    };
}

test "a snapshot of written text" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    session.stream.nextSlice("hi");
    var buf: testBuffer(10, 3) align(4) = undefined;
    try std.testing.expectEqual(3, try fillSnapshot(session, &buf));
    // Nothing new: nothing to draw.
    try std.testing.expectEqual(0, try fillSnapshot(session, &buf));
}

test "every row again when the default colors change" {
    // OSC 11 and reverse video change what every row's blank cells are drawn
    // in, without marking a row: the view drew them in the old colors.
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.stream.nextSlice("a\r\nb\r\nc");
    _ = try fillSnapshot(session, &buf);

    session.stream.nextSlice("\x1b]11;#5f0000\x07");
    try std.testing.expectEqual(3, try fillSnapshot(session, &buf));
    try std.testing.expectEqual(2, headerOf(&buf).dirty);
    try std.testing.expectEqual(argb(.{ .r = 0x5f, .g = 0, .b = 0 }), headerOf(&buf).bg);
    try std.testing.expectEqual(0, try fillSnapshot(session, &buf));

    session.stream.nextSlice("\x1b[?5h");
    try std.testing.expectEqual(3, try fillSnapshot(session, &buf));
    try std.testing.expectEqual(argb(.{ .r = 0x5f, .g = 0, .b = 0 }), headerOf(&buf).fg);
}

test "the cursor on a wide character covers it, from its first cell" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    // 中 takes columns 0 and 1, 文 2 and 3.
    session.stream.nextSlice("中文");
    for ([_]struct { col: u8, x: i32, wide: i32 }{
        .{ .col = 1, .x = 0, .wide = 1 }, // on 中
        .{ .col = 2, .x = 0, .wide = 1 }, // on 中's right half
        .{ .col = 5, .x = 4, .wide = 0 }, // after both
    }) |case| {
        var move: [16]u8 = undefined;
        session.stream.nextSlice(try std.fmt.bufPrint(&move, "\x1b[1;{d}H", .{case.col}));
        _ = try fillSnapshot(session, &buf);
        try std.testing.expectEqual(case.x, headerOf(&buf).cursor_x);
        try std.testing.expectEqual(case.wide, headerOf(&buf).cursor_wide);
    }
}

test "the snapshot carries the cell under the cursor" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.stream.nextSlice("\x1b[1mW\x1b[0m中\x1b[1;1H");
    _ = try fillSnapshot(session, &buf);
    const at = struct {
        fn f(b: []const u8, i: usize) i32 {
            return std.mem.readInt(i32, b[i * 4 ..][0..4], .little);
        }
    }.f;
    try std.testing.expectEqual(flag_bold, at(&buf, 15));
    try std.testing.expectEqual(1, at(&buf, 16));
    try std.testing.expectEqual('W', at(&buf, 17) & 0xFFFF);

    // On 中's right half: 中.
    session.stream.nextSlice("\x1b[1;3H");
    _ = try fillSnapshot(session, &buf);
    try std.testing.expectEqual(1, at(&buf, 16));
    try std.testing.expectEqual('中', at(&buf, 17) & 0xFFFF);
}

test "an emoji with VS16 is two cells wide, also after a reset" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    session.stream.nextSlice("❤️X");
    try std.testing.expectEqual(3, session.term.screens.active.cursor.x);
    session.stream.nextSlice("\x1bc❤️X");
    try std.testing.expectEqual(3, session.term.screens.active.cursor.x);
}

test "an overlined cell says so" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.stream.nextSlice("\x1b[53mO\x1b[55mx");
    _ = try fillSnapshot(session, &buf);
    const cells = header_bytes + row_header_bytes;
    const flags = struct {
        fn f(b: []const u8, col: usize) u16 {
            return std.mem.readInt(u16, b[cells + col * cell_record_bytes + 12 ..][0..2], .little);
        }
    }.f;
    try std.testing.expect(flags(&buf, 0) & flag_overline != 0);
    try std.testing.expect(flags(&buf, 1) & flag_overline == 0);
}

test "a cell's text and underline colors, true color or the palette's" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    // RGB text, palette text (196: the cube's red), default text, then
    // underlines colored as RGB and from the palette (46: the cube's green).
    session.stream.nextSlice("\x1b[38;2;1;2;3mA\x1b[38;5;196mB\x1b[0mC\x1b[4;58;2;9;8;7mU\x1b[58;5;46mV");
    _ = try fillSnapshot(session, &buf);
    const cells = header_bytes + row_header_bytes;
    const at = struct {
        fn i32At(b: []const u8, col: usize, field: usize) i32 {
            return std.mem.readInt(i32, b[cells + col * cell_record_bytes + field ..][0..4], .little);
        }
        fn flags(b: []const u8, col: usize) u16 {
            return std.mem.readInt(u16, b[cells + col * cell_record_bytes + 12 ..][0..2], .little);
        }
    };
    try std.testing.expectEqual(argb(.{ .r = 1, .g = 2, .b = 3 }), at.i32At(&buf, 0, 0));
    try std.testing.expectEqual(argb(.{ .r = 255, .g = 0, .b = 0 }), at.i32At(&buf, 1, 0));
    try std.testing.expect(at.flags(&buf, 1) & flag_fg_default == 0);
    try std.testing.expect(at.flags(&buf, 2) & flag_fg_default != 0);
    try std.testing.expectEqual(0, at.i32At(&buf, 2, 16));
    try std.testing.expectEqual(argb(.{ .r = 9, .g = 8, .b = 7 }), at.i32At(&buf, 3, 16));
    try std.testing.expectEqual(argb(.{ .r = 0, .g = 255, .b = 0 }), at.i32At(&buf, 4, 16));
}

test "a theme without colors goes back to the built-in ones" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.setTheme(.{ .r = 1, .g = 2, .b = 3 }, .{ .r = 4, .g = 5, .b = 6 }, null, vt.color.default);
    _ = try fillSnapshot(session, &buf);
    try std.testing.expectEqual(argb(.{ .r = 4, .g = 5, .b = 6 }), headerOf(&buf).bg);
    session.setTheme(null, null, null, vt.color.default);
    _ = try fillSnapshot(session, &buf);
    try std.testing.expectEqual(argb(default_background), headerOf(&buf).bg);
    try std.testing.expectEqual(argb(default_foreground), headerOf(&buf).fg);
}

test "the cursor moving sends the rows it left and moved to" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.stream.nextSlice("a\r\nb\r\nc\x1b[1;1H");
    _ = try fillSnapshot(session, &buf);
    try std.testing.expectEqual(0, try fillSnapshot(session, &buf));
    // Row 0 to row 2: those two, not row 1.
    session.stream.nextSlice("\x1b[3;1H");
    try std.testing.expectEqual(2, try fillSnapshot(session, &buf));
    try std.testing.expectEqual(0, std.mem.readInt(i32, buf[header_bytes..][0..4], .little));
}

/// The row records of the last snapshot in `buf`: each one's row and where it moved from.
fn recordsOf(buf: []const u8, cols: usize) ![8][2]i32 {
    var out: [8][2]i32 = @splat(.{ -9, -9 });
    const count: usize = @intCast(headerOf(buf).rows);
    var offset: usize = header_bytes;
    for (0..count) |i| {
        const at = struct {
            fn f(b: []const u8, o: usize) i32 {
                return std.mem.readInt(i32, b[o..][0..4], .little);
            }
        }.f;
        out[i] = .{ at(buf, offset), at(buf, offset + 16) };
        if (out[i][1] >= 0) {
            offset += row_header_bytes;
        } else {
            const units: usize = @intCast(at(buf, offset + 12));
            offset += row_header_bytes + cols * cell_record_bytes + units * 2;
            if (offset % 4 != 0) offset += 2;
        }
    }
    return out;
}

test "scrolled rows come as where they were, not again" {
    const session = try create(10, 4, 100);
    defer destroy(session);
    var buf: testBuffer(10, 4) align(4) = undefined;
    session.stream.nextSlice("1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8");
    _ = try fillSnapshot(session, &buf);
    // The viewport up a row: what was on rows 0-2 is now on 1-3, and row 0 is
    // new. (Row 3 had the cursor, which scrolled off: drawn again.)
    session.term.scrollViewport(.{ .delta = -1 });
    try std.testing.expectEqual(4, try fillSnapshot(session, &buf));
    const records = try recordsOf(&buf, 10);
    try std.testing.expectEqual([2]i32{ 0, -1 }, records[0]);
    try std.testing.expectEqual([2]i32{ 1, 0 }, records[1]);
    try std.testing.expectEqual([2]i32{ 2, 1 }, records[2]);
    try std.testing.expectEqual([2]i32{ 3, -1 }, records[3]);
    // And down again: all but the cursor's two rows moved back.
    session.term.scrollViewport(.{ .delta = 1 });
    _ = try fillSnapshot(session, &buf);
    const back = try recordsOf(&buf, 10);
    try std.testing.expectEqual([2]i32{ 0, 1 }, back[0]);
    try std.testing.expectEqual([2]i32{ 1, 2 }, back[1]);
}

test "a row rewritten as it was isn't sent again" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.stream.nextSlice("abc\r\nxyz\x1b[3;1H");
    _ = try fillSnapshot(session, &buf);
    // A TUI redrawing row 0 as it was, and changing row 1.
    session.stream.nextSlice("\x1b[1;1Habc\x1b[2;1Hxyw\x1b[3;1H");
    try std.testing.expectEqual(1, try fillSnapshot(session, &buf));
    try std.testing.expectEqual([2]i32{ 1, -1 }, (try recordsOf(&buf, 10))[0]);
}

test "after the view lost its drawings, every row again" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.stream.nextSlice("a");
    _ = try fillSnapshot(session, &buf);
    session.dirtyAll();
    try std.testing.expectEqual(3, try fillSnapshot(session, &buf));
}

test "a selection's end moving sends only the rows it changed" {
    const session = try create(10, 5, 100);
    defer destroy(session);
    var buf: testBuffer(10, 5) align(4) = undefined;
    session.stream.nextSlice("a\r\nb\r\nc\r\nd\r\ne\x1b[1;1H");
    _ = try fillSnapshot(session, &buf);
    // Rows 1 to 3 selected, then the end dragged on to row 4.
    _ = session.select(.init(session.viewportPin(0, 1).?, session.viewportPin(5, 3).?, false));
    try std.testing.expectEqual(3, try fillSnapshot(session, &buf));
    _ = session.select(.init(session.viewportPin(0, 1).?, session.viewportPin(5, 4).?, false));
    // Row 3 (now selected to its end) and row 4.
    try std.testing.expectEqual(2, try fillSnapshot(session, &buf));
    session.reselect();
    session.term.screens.active.clearSelection();
    try std.testing.expectEqual(4, try fillSnapshot(session, &buf));
}

test "a link and the cells it covers, wrapped or not" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    // "see https://e.co/abc" wraps onto row 1 at column 10.
    session.stream.nextSlice("see https://e.co/abc now");
    const link = (try linkAt(session, 6, 0)).?;
    defer alloc.free(link.uri);
    try std.testing.expectEqualStrings("https://e.co/abc", link.uri);
    try std.testing.expectEqual(Link.Cell{ .x = 4, .y = 0 }, link.first);
    try std.testing.expectEqual(Link.Cell{ .x = 9, .y = 1 }, link.last);
    try std.testing.expectEqual(null, try linkAt(session, 1, 0));

    // An OSC 8 hyperlink: its cells.
    session.stream.nextSlice("\x1b[3;1Hx \x1b]8;;https://h.co\x1b\\here\x1b]8;;\x1b\\ y");
    const osc = (try linkAt(session, 3, 2)).?;
    defer alloc.free(osc.uri);
    try std.testing.expectEqualStrings("https://h.co", osc.uri);
    try std.testing.expectEqual(Link.Cell{ .x = 2, .y = 2 }, osc.first);
    try std.testing.expectEqual(Link.Cell{ .x = 5, .y = 2 }, osc.last);
}

test "the snapshot says where the viewport is in the scrollback" {
    const session = try create(10, 3, 100);
    defer destroy(session);
    var buf: testBuffer(10, 3) align(4) = undefined;
    session.stream.nextSlice("1\r\n2\r\n3\r\n4\r\n5");
    session.term.scrollViewport(.{ .delta = -1 });
    _ = try fillSnapshot(session, &buf);
    const at = struct {
        fn f(b: []const u8, i: usize) i32 {
            return std.mem.readInt(i32, b[i * 4 ..][0..4], .little);
        }
    }.f;
    try std.testing.expectEqual(5, at(&buf, 19));
    try std.testing.expectEqual(1, at(&buf, 20));
    try std.testing.expectEqual(3, at(&buf, 21));
}
