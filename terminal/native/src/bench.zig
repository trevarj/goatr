//! goatr-bench: times what the view asks of the session, frame by frame, on
//! this machine. As ghostty's benchmarks (src/benchmark): it reads input
//! made beforehand with ghostty-gen and does the work; hyperfine times the
//! runs and compares builds. `mise run bench` does all of it.
//!
//!     goatr-bench <mode> [--data=<file>] [--cols=120] [--rows=50] [--loops=200]
//!
//! Modes (the view's paths):
//!   stream  writes the data in 4 KB chunks, a snapshot after each, as the
//!           view draws a frame while output floods in
//!   full    every row again, `loops` times (a resize, a theme)
//!   scroll  the viewport up a row and down again, a snapshot each (a fling)
//!   select  a selection's end moving across the screen, a snapshot each (a
//!           handle dragged)

const std = @import("std");
const session_mod = @import("session.zig");

const Mode = enum { stream, full, scroll, select };

pub fn main(init: std.process.Init) !void {
    const gpa = init.gpa;
    const args = try init.minimal.args.toSlice(init.arena.allocator());
    if (args.len < 2) return error.NoMode;
    const mode = std.meta.stringToEnum(Mode, args[1]) orelse return error.UnknownMode;

    var data_path: ?[]const u8 = null;
    var cols: i32 = 120;
    var rows: i32 = 50;
    var loops: usize = 200;
    for (args[2..]) |arg| {
        if (std.mem.startsWith(u8, arg, "--data=")) data_path = arg[7..];
        if (std.mem.startsWith(u8, arg, "--cols=")) cols = try std.fmt.parseInt(i32, arg[7..], 10);
        if (std.mem.startsWith(u8, arg, "--rows=")) rows = try std.fmt.parseInt(i32, arg[7..], 10);
        if (std.mem.startsWith(u8, arg, "--loops=")) loops = try std.fmt.parseInt(usize, arg[8..], 10);
    }
    const data = if (data_path) |p|
        try std.Io.Dir.cwd().readFileAlloc(init.io, p, gpa, .unlimited)
    else
        &.{};
    defer gpa.free(data);

    const s = try session_mod.create(cols, rows, 10_000);
    defer session_mod.destroy(s);
    // A buffer as the view sizes it.
    const ucols: usize = @intCast(cols);
    const urows: usize = @intCast(rows);
    const buf = try gpa.alignedAlloc(u8, .@"4", session_mod.bufferSize(ucols, urows));
    defer gpa.free(buf);

    var drawn: usize = 0;
    switch (mode) {
        .stream => {
            var at: usize = 0;
            while (at < data.len) : (at += chunk) {
                s.stream.nextSlice(data[at..@min(data.len, at + chunk)]);
                drawn += @intCast(try session_mod.fillSnapshot(s, buf));
            }
        },
        .full, .scroll, .select => {
            s.stream.nextSlice(data);
            _ = try session_mod.fillSnapshot(s, buf);
            for (0..loops) |i| {
                switch (mode) {
                    .full => s.dirtyAll(),
                    .scroll => s.term.scrollViewport(.{ .delta = if (i % 2 == 0) -1 else 1 }),
                    .select => {
                        const end_col: i32 = @intCast(i % ucols);
                        const end_row: i32 = @intCast((i / ucols) % urows);
                        const start = s.viewportPin(0, 0) orelse continue;
                        const end = s.viewportPin(end_col, end_row) orelse continue;
                        _ = s.select(.init(start, end, false));
                    },
                    .stream => unreachable,
                }
                drawn += @intCast(try session_mod.fillSnapshot(s, buf));
            }
        },
    }
    // What was drawn, so none of it is optimized away (and to compare builds' work).
    var out: [64]u8 = undefined;
    const line = try std.fmt.bufPrint(&out, "{s}: {d} rows drawn\n", .{ @tagName(mode), drawn });
    try std.Io.File.stderr().writeStreamingAll(init.io, line);
}

// Bytes written between snapshots, about what a frame's worth of output is
// when the view parses what came in since the last.
const chunk = 4096;
