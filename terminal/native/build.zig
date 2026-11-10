//! libherdrterminal.so: the JNI bridge (src/jni.zig) with ghostty's
//! `ghostty-vt` Zig module compiled in, for one Android ABI. Gradle runs
//! `zig build -Dtarget=<abi target>` per ABI, with ANDROID_NDK_HOME set.
//!
//! `zig build test` runs the session's tests (src/session.zig) on this
//! machine, and `zig build bench` builds goatr-bench (src/bench.zig) for it;
//! neither needs the NDK.

const std = @import("std");
const android_ndk = @import("android_ndk");

pub fn build(b: *std.Build) !void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    if (target.result.abi.isAndroid()) try library(b, target, optimize);

    // The session, without JNI, on this machine.
    const host = b.graph.host;
    const ghostty = b.dependency("ghostty", .{ .target = host, .optimize = optimize, .simd = false });
    const session = b.createModule(.{
        .root_source_file = b.path("src/session.zig"),
        .target = host,
        .optimize = optimize,
        .link_libc = true,
    });
    session.addImport("ghostty-vt", ghostty.module("ghostty-vt"));
    const tests = b.addTest(.{ .root_module = session });
    b.step("test", "Run the session's tests").dependOn(&b.addRunArtifact(tests).step);

    // goatr-bench (src/bench.zig), always optimized as the app's library is.
    const fast = b.dependency("ghostty", .{ .target = host, .optimize = .ReleaseFast, .simd = false });
    const bench = b.createModule(.{
        .root_source_file = b.path("src/bench.zig"),
        .target = host,
        .optimize = .ReleaseFast,
        .link_libc = true,
    });
    bench.addImport("ghostty-vt", fast.module("ghostty-vt"));
    const exe = b.addExecutable(.{ .name = "goatr-bench", .root_module = bench });
    b.step("bench", "Build goatr-bench").dependOn(&b.addInstallArtifact(exe, .{}).step);
}

/// libherdrterminal.so for an Android target.
fn library(b: *std.Build, target: std.Build.ResolvedTarget, optimize: std.builtin.OptimizeMode) !void {
    // JNI and Android headers, translated from the NDK's sysroot.
    const sysroot = try ndkSysroot(b);
    const triple = switch (target.result.cpu.arch) {
        .aarch64 => "aarch64-linux-android",
        .x86_64 => "x86_64-linux-android",
        else => return error.UnsupportedAbi,
    };
    const headers = b.addTranslateC(.{
        .root_source_file = b.path("src/c.h"),
        .target = target,
        .optimize = optimize,
        .link_libc = false,
    });
    headers.addSystemIncludePath(.{ .cwd_relative = b.pathJoin(&.{ sysroot, "usr", "include" }) });
    headers.addSystemIncludePath(.{ .cwd_relative = b.pathJoin(&.{ sysroot, "usr", "include", triple }) });

    // -Dsimd=false drops ghostty's C++ SIMD paths: plain Zig, nothing else to link.
    const ghostty = b.dependency("ghostty", .{ .target = target, .optimize = optimize, .simd = false });

    const mod = b.createModule(.{
        .root_source_file = b.path("src/jni.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
        .strip = optimize != .debug,
    });
    mod.addImport("c", headers.createModule());
    mod.addImport("ghostty-vt", ghostty.module("ghostty-vt"));
    // liblog lives under the API level's own directory.
    const api = target.result.os.version_range.linux.android;
    mod.addLibraryPath(.{ .cwd_relative = b.pathJoin(&.{ sysroot, "usr", "lib", triple, b.fmt("{d}", .{api}) }) });
    mod.linkSystemLibrary("log", .{});

    const lib = b.addLibrary(.{
        .name = "herdrterminal",
        .linkage = .dynamic,
        .root_module = mod,
    });
    try android_ndk.addPaths(b, lib);
    // Android 15+ devices require 16 KB page alignment.
    lib.link_z_max_page_size = 16384;
    b.installArtifact(lib);
}

fn ndkSysroot(b: *std.Build) ![]const u8 {
    const ndk = b.graph.environ_map.get("ANDROID_NDK_HOME") orelse return error.AndroidNdkHomeNotSet;
    return b.pathJoin(&.{ ndk, "toolchains", "llvm", "prebuilt", "linux-x86_64", "sysroot" });
}
