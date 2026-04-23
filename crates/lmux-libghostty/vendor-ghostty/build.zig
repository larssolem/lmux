const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const ghostty = b.dependency("ghostty", .{
        .target = target,
        .optimize = optimize,
        .simd = false,
    });

    b.installArtifact(ghostty.artifact("ghostty-vt-static"));

    b.installDirectory(.{
        .source_dir = ghostty.path("include"),
        .install_dir = .header,
        .install_subdir = "",
    });
}
