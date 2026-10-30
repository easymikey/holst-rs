//! Local release: builds the assets of `.github/workflows/release.yml` on one Mac, for when the
//! GitHub runners are down. See "Releasing without GitHub runners" in CONTRIBUTING.md.
//!
//!     zig run scripts/release.zig -- <tag>              # build, pack, checksum into dist/
//!     zig run scripts/release.zig -- <tag> --publish    # also create the GitHub release
//!
//! Zig 0.17.0 exactly: the script stops on any other version.
//!
//! <tag> is `v` plus the holst-cli Cargo version; the working tree must be clean. Output goes to
//! $DIST (default `dist`), Cargo builds into $CARGO_TARGET_DIR (default `target`).
//!
//! Needs, all checked first (it prints what is missing and installs nothing): rustup with the five
//! targets, zig, cargo-run-bin (runs the cargo-zigbuild and cargo-xwin pinned in Cargo.toml),
//! `clang-cl` on PATH, tar, zip, shasum, file, and gh for --publish.
//!
//! One archive per program and target: `holst-cli-<target>` and `holst-mcp-<target>`, ten in all.
//! The Linux targets are static musl builds, so the archives run on any distribution.
//!
//! `--publish` is the only step that touches GitHub: the local tag must point at HEAD and exist on
//! the remote; it writes the release notes and runs `gh release create --verify-tag`.
//! crates.io and Homebrew are not published; the commands are printed at the end.

const std = @import("std");
const builtin = @import("builtin");

const Io = std.Io;
const Allocator = std.mem.Allocator;

const pinned_zig: std.SemanticVersion = .{ .major = 0, .minor = 17, .patch = 0 };

const Os = enum { macos, linux, windows };

const Target = struct {
    triple: []const u8,
    os: Os,

    fn exeSuffix(t: Target) []const u8 {
        return if (t.os == .windows) ".exe" else "";
    }

    fn archiveName(t: Target, program: Program, arena: Allocator) ![]u8 {
        const ext = if (t.os == .windows) "zip" else "tar.gz";
        return std.fmt.allocPrint(arena, "{s}-{s}.{s}", .{ program.package, t.triple, ext });
    }
};

/// A Cargo package and the binary it builds; each gets its own archive.
const Program = struct { package: []const u8, binary: []const u8 };

const programs = [_]Program{
    .{ .package = "holst-cli", .binary = "holst" },
    .{ .package = "holst-mcp", .binary = "holst-mcp" },
};

const targets = [_]Target{
    .{ .triple = "aarch64-apple-darwin", .os = .macos },
    .{ .triple = "x86_64-apple-darwin", .os = .macos },
    .{ .triple = "x86_64-unknown-linux-musl", .os = .linux },
    .{ .triple = "aarch64-unknown-linux-musl", .os = .linux },
    .{ .triple = "x86_64-pc-windows-msvc", .os = .windows },
};

const extra_files = [_][]const u8{ "LICENSE-MIT", "LICENSE-APACHE", "readme.md" };

const Tool = struct { name: []const u8, install: []const u8 };

const required_tools = [_]Tool{
    .{ .name = "rustup", .install = "https://rustup.rs" },
    .{ .name = "cargo", .install = "https://rustup.rs" },
    .{ .name = "zig", .install = "brew install zig" },
    .{ .name = "cargo-bin", .install = "cargo install --locked cargo-run-bin" },
    .{ .name = "clang-cl", .install = "brew install llvm, then put $(brew --prefix llvm)/bin on PATH (cargo-xwin compiles C with it)" },
    .{ .name = "tar", .install = "ships with macOS" },
    .{ .name = "zip", .install = "ships with macOS" },
    .{ .name = "shasum", .install = "ships with macOS" },
    .{ .name = "file", .install = "ships with macOS" },
    .{ .name = "git", .install = "ships with the Xcode command line tools" },
};

const usage = "usage: zig run scripts/release.zig -- <tag> [--publish]";

const Release = struct {
    io: Io,
    arena: Allocator,
    /// Environment handed to every child; `publish` and `pack` add to it.
    env: *std.process.Environ.Map,
    tag: []const u8,
    publish_requested: bool,
    dist: []const u8,
    cargo_target_dir: []const u8,
};

pub fn main(init: std.process.Init) !void {
    if (builtin.zig_version.order(pinned_zig) != .eq) {
        fail("this script is written for Zig {f}, found {f}; install that version, or update scripts/release.zig on purpose", .{ pinned_zig, builtin.zig_version });
    }

    const arena = init.arena.allocator();
    const args = try init.minimal.args.toSlice(arena);
    var release = Release{
        .io = init.io,
        .arena = arena,
        .env = init.environ_map,
        .tag = parseTag(args),
        .publish_requested = args.len == 3,
        .dist = init.environ_map.get("DIST") orelse "dist",
        .cargo_target_dir = init.environ_map.get("CARGO_TARGET_DIR") orelse "target",
    };

    try checkTools(&release);
    try enterRepoRoot(&release);
    try checkTag(&release);

    try Io.Dir.cwd().deleteTree(release.io, release.dist);
    try Io.Dir.cwd().createDirPath(release.io, release.dist);
    for (targets) |target| {
        say("== {s}", .{target.triple});
        try build(&release, target);
        try pack(&release, target);
    }
    try checksums(&release);
    if (release.publish_requested) try publish(&release);

    try finish(&release);
}

/// Reads `<tag> [--publish]`; anything else prints the usage and exits with 2.
fn parseTag(args: []const [:0]const u8) []const u8 {
    const wrong_count = args.len < 2 or args.len > 3;
    if (wrong_count or (args.len == 3 and !std.mem.eql(u8, args[2], "--publish"))) {
        std.debug.print("{s}\n", .{usage});
        std.process.exit(2);
    }
    return args[1];
}

/// Step 1: every tool the later steps run must exist; list the missing ones with how to get them.
fn checkTools(r: *Release) !void {
    var missing: std.ArrayList([]const u8) = .empty;
    for (required_tools) |tool| {
        if (!onPath(r, tool.name)) try missing.append(r.arena, try std.fmt.allocPrint(r.arena, "{s}: {s}", .{ tool.name, tool.install }));
    }
    if (r.publish_requested and !onPath(r, "gh")) try missing.append(r.arena, "gh: brew install gh");

    if (onPath(r, "rustup")) {
        const installed = try capture(r, &.{ "rustup", "target", "list", "--installed" });
        for (targets) |target| {
            if (!hasLine(installed, target.triple)) {
                try missing.append(r.arena, try std.fmt.allocPrint(r.arena, "rust target {s}: rustup target add {s}", .{ target.triple, target.triple }));
            }
        }
    }

    if (missing.items.len != 0) {
        std.debug.print("missing tools, install them and rerun:\n", .{});
        for (missing.items) |line| std.debug.print("  {s}\n", .{line});
        std.process.exit(1);
    }
}

/// Step 2: work from the repository root, so `dist/` and the Cargo paths do not depend on where the script was started.
fn enterRepoRoot(r: *Release) !void {
    const root = try capture(r, &.{ "git", "rev-parse", "--show-toplevel" });
    try std.process.setCurrentPath(r.io, root);
}

/// Step 3: the tag must be `v` plus the holst-cli version, the tree clean, and for --publish the local tag must sit on HEAD.
fn checkTag(r: *Release) !void {
    const pkgid = try capture(r, &.{ "cargo", "pkgid", "-p", "holst-cli" });
    // `path+file:///repo/crates/holst-cli#0.3.0` or `...#holst-cli@0.3.0`
    const version_start = std.mem.lastIndexOfAny(u8, pkgid, "#@") orelse fail("cannot read the version from `cargo pkgid`: {s}", .{pkgid});
    const version = pkgid[version_start + 1 ..];
    if (!std.mem.eql(u8, r.tag, try std.fmt.allocPrint(r.arena, "v{s}", .{version}))) {
        fail("tag {s} does not match the Cargo version {s}", .{ r.tag, version });
    }

    if ((try capture(r, &.{ "git", "status", "--porcelain" })).len != 0) fail("the working tree is not clean", .{});

    if (!r.publish_requested) return;
    const tag_commit = try capture(r, &.{ "git", "rev-parse", try std.fmt.allocPrint(r.arena, "{s}^{{commit}}", .{r.tag}) });
    const head = try capture(r, &.{ "git", "rev-parse", "HEAD" });
    if (!std.mem.eql(u8, tag_commit, head)) fail("the local tag {s} must exist and point at HEAD", .{r.tag});
}

/// Step 4: release build of both binaries for one target; macOS native, Linux (static musl) with zigbuild, Windows with xwin.
fn build(r: *Release, target: Target) !void {
    const command: []const []const u8 = switch (target.os) {
        .macos => &.{ "cargo", "build" },
        .linux => &.{ "cargo", "bin", "cargo-zigbuild" },
        .windows => &.{ "cargo", "bin", "cargo-xwin", "build" },
    };
    const flags: []const []const u8 = &.{ "--release", "--locked", "-p", "holst-cli", "-p", "holst-mcp", "--target", target.triple };
    try runVisible(r, try std.mem.concat(r.arena, []const u8, &.{ command, flags }), .inherit);
    if (target.os == .linux) try checkStatic(r, target);
}

/// A Linux archive must run on any distribution: `file` has to call both binaries statically linked.
fn checkStatic(r: *Release, target: Target) !void {
    for (programs) |program| {
        const path = try std.fmt.allocPrint(r.arena, "{s}/{s}/release/{s}", .{ r.cargo_target_dir, target.triple, program.binary });
        const info = try capture(r, &.{ "file", path });
        say("{s}", .{info});
        if (std.mem.indexOf(u8, info, "statically linked") == null and std.mem.indexOf(u8, info, "static-pie linked") == null) {
            fail("{s} is not statically linked", .{path});
        }
    }
}

/// Step 5: one archive per program, flat, as the workflow does: the binary with the license files and readme.
fn pack(r: *Release, target: Target) !void {
    for (programs) |program| {
        const cwd = Io.Dir.cwd();
        const stage = try std.fmt.allocPrint(r.arena, "{s}/stage-{s}-{s}", .{ r.dist, program.package, target.triple });
        try cwd.createDirPath(r.io, stage);

        const binary = try std.fmt.allocPrint(r.arena, "{s}{s}", .{ program.binary, target.exeSuffix() });
        var names: std.ArrayList([]const u8) = .empty;
        try names.append(r.arena, binary);
        try names.appendSlice(r.arena, &extra_files);

        const built = try std.fmt.allocPrint(r.arena, "{s}/{s}/release/{s}", .{ r.cargo_target_dir, target.triple, binary });
        try cwd.copyFile(built, cwd, try std.fmt.allocPrint(r.arena, "{s}/{s}", .{ stage, binary }), r.io, .{});
        for (extra_files) |file| {
            try cwd.copyFile(file, cwd, try std.fmt.allocPrint(r.arena, "{s}/{s}", .{ stage, file }), r.io, .{});
        }

        const archive = try target.archiveName(program, r.arena);
        switch (target.os) {
            .windows => {
                // zip runs inside the stage dir so the archive is flat; the std library cannot write zip
                const out = try std.fmt.allocPrint(r.arena, "../{s}", .{archive});
                const argv = try std.mem.concat(r.arena, []const u8, &.{ &.{ "zip", "-q", "-X", out }, names.items });
                _ = try captureIn(r, argv, .{ .path = stage });
            },
            .macos, .linux => {
                // COPYFILE_DISABLE stops macOS tar from adding `._` files
                try r.env.put("COPYFILE_DISABLE", "1");
                const out = try std.fmt.allocPrint(r.arena, "{s}/{s}", .{ r.dist, archive });
                const argv = try std.mem.concat(r.arena, []const u8, &.{ &.{ "tar", "-czf", out, "-C", stage }, names.items });
                _ = try captureIn(r, argv, .inherit);
            },
        }
    }
}

/// Step 6: `checksums.txt` in the format of `sha256sum`, one line per archive.
fn checksums(r: *Release) !void {
    var argv: std.ArrayList([]const u8) = .empty;
    try argv.appendSlice(r.arena, &.{ "shasum", "-a", "256" });
    for (programs) |program| {
        for (targets) |target| try argv.append(r.arena, try target.archiveName(program, r.arena));
    }

    const sums = try captureIn(r, argv.items, .{ .path = r.dist });
    const path = try std.fmt.allocPrint(r.arena, "{s}/checksums.txt", .{r.dist});
    try Io.Dir.cwd().writeFile(r.io, .{ .sub_path = path, .data = sums });
}

/// Step 7 (--publish only): release notes like the workflow's, then `gh release create`.
fn publish(r: *Release) !void {
    const repo = try capture(r, &.{ "gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner" });
    const previous = try previousTag(r);
    const range = if (previous.len != 0) try std.fmt.allocPrint(r.arena, "{s}..{s}", .{ previous, r.tag }) else r.tag;

    var notes: Io.Writer.Allocating = .init(r.arena);
    const w = &notes.writer;
    try w.writeAll("## What's Changed\n\n");
    const log = try capture(r, &.{ "git", "log", range, "--pretty=format:%H|%s|%an" });
    var lines = std.mem.splitScalar(u8, log, '\n');
    while (lines.next()) |line| {
        if (line.len == 0) continue;
        // the subject may contain `|`; the hash never does and the author name is taken after the last one
        const first = std.mem.indexOfScalar(u8, line, '|') orelse continue;
        const last = std.mem.lastIndexOfScalar(u8, line, '|') orelse continue;
        const hash = line[0..first];
        const title = line[first + 1 .. last];
        const author = line[last + 1 ..];
        const login = authorLogin(r, repo, hash);
        if (login.len != 0) {
            try w.print("- {s} by {s} ([@{s}](https://github.com/{s})) ", .{ title, author, login, login });
        } else {
            try w.print("- {s} by {s} ", .{ title, author });
        }
        try w.print("([`{s}`](https://github.com/{s}/commit/{s}))\n", .{ hash[0..7], repo, hash });
    }
    const sums_path = try std.fmt.allocPrint(r.arena, "{s}/checksums.txt", .{r.dist});
    const sums = try Io.Dir.cwd().readFileAlloc(r.io, sums_path, r.arena, .limited(1 << 20));
    try w.print("\n## Checksums (SHA256)\n\n```\n{s}```\n", .{sums});
    if (previous.len != 0) try w.print("\n**Full Changelog**: https://github.com/{s}/compare/{s}...{s}\n", .{ repo, previous, r.tag });

    const notes_path = try std.fmt.allocPrint(r.arena, "{s}/release-notes.md", .{r.dist});
    try Io.Dir.cwd().writeFile(r.io, .{ .sub_path = notes_path, .data = notes.written() });

    var argv: std.ArrayList([]const u8) = .empty;
    try argv.appendSlice(r.arena, &.{ "gh", "release", "create", r.tag });
    for (programs) |program| {
        for (targets) |target| try argv.append(r.arena, try std.fmt.allocPrint(r.arena, "{s}/{s}", .{ r.dist, try target.archiveName(program, r.arena) }));
    }
    try argv.append(r.arena, sums_path);
    try argv.appendSlice(r.arena, &.{ "--verify-tag", "--title", r.tag, "--notes-file", notes_path, "--latest" });
    try r.env.put("GH_REPO", repo);
    try runVisible(r, argv.items, .inherit);
}

/// The newest `v<digit>...` tag reachable before `r.tag`, or empty when there is none.
fn previousTag(r: *Release) ![]const u8 {
    const before = try std.fmt.allocPrint(r.arena, "{s}^", .{r.tag});
    const tags = try capture(r, &.{ "git", "tag", "--merged", before, "--sort=-version:refname" });
    var lines = std.mem.splitScalar(u8, tags, '\n');
    while (lines.next()) |line| {
        if (line.len >= 2 and line[0] == 'v' and std.ascii.isDigit(line[1])) return line;
    }
    return "";
}

/// GitHub login of a commit author, or empty when GitHub does not know it (a failed lookup is not an error).
fn authorLogin(r: *Release, repo: []const u8, hash: []const u8) []const u8 {
    const path = std.fmt.allocPrint(r.arena, "repos/{s}/commits/{s}", .{ repo, hash }) catch return "";
    const result = std.process.run(r.arena, r.io, .{
        .argv = &.{ "gh", "api", path, "--jq", ".author.login // empty" },
    }) catch return "";
    if (!result.term.success()) return "";
    return std.mem.trim(u8, result.stdout, " \t\r\n");
}

/// Last step: list the assets and print what the workflow's crates.io and Homebrew jobs would still do.
fn finish(r: *Release) !void {
    say("\nassets in {s}:", .{r.dist});
    for (programs) |program| {
        for (targets) |target| say("  {s}/{s}", .{ r.dist, try target.archiveName(program, r.arena) });
    }
    say("  {s}/checksums.txt", .{r.dist});
    say(
        \\
        \\Not done by this script (the workflow's crates and homebrew jobs):
        \\  crates.io: for name in holst-macros holst-form holst-core holst-cli holst-mcp, cargo publish -p $name --locked
        \\  homebrew:  in easymikey/homebrew-holst set Formula/holst-cli.rb and Formula/holst-mcp.rb to version {s},
        \\             url https://github.com/easymikey/holst-rs/releases/download/{s}/<package>-<target>.tar.gz
        \\             and the sha256 of each program's four macOS and Linux archives from {s}/checksums.txt
    , .{ r.tag[1..], r.tag, r.dist });
}

// Helpers below: running tools and reporting.

/// Runs a command, stdout and stderr captured; returns trimmed stdout. A failure names the command and shows its stderr.
fn capture(r: *Release, argv: []const []const u8) ![]const u8 {
    return std.mem.trim(u8, try captureIn(r, argv, .inherit), " \t\r\n");
}

/// Like `capture` in `cwd`, but returns stdout untrimmed.
fn captureIn(r: *Release, argv: []const []const u8, cwd: std.process.Child.Cwd) ![]const u8 {
    const command = try std.mem.join(r.arena, " ", argv);
    const result = std.process.run(r.arena, r.io, .{ .argv = argv, .cwd = cwd, .environ_map = r.env }) catch |err|
        fail("cannot run `{s}`: {t}", .{ command, err });
    if (!result.term.success()) fail("`{s}` {f}:\n{s}", .{ command, result.term, result.stderr });
    return result.stdout;
}

/// Runs a command with its output on the terminal (builds take minutes; their output is the progress).
fn runVisible(r: *Release, argv: []const []const u8, cwd: std.process.Child.Cwd) !void {
    const command = try std.mem.join(r.arena, " ", argv);
    var child = std.process.spawn(r.io, .{ .argv = argv, .cwd = cwd, .environ_map = r.env }) catch |err|
        fail("cannot run `{s}`: {t}", .{ command, err });
    const term = try child.wait(r.io);
    if (!term.success()) fail("`{s}` {f}", .{ command, term });
}

/// True when an executable called `name` is in a directory of PATH.
fn onPath(r: *Release, name: []const u8) bool {
    var dirs = std.mem.tokenizeScalar(u8, r.env.get("PATH") orelse "", ':');
    while (dirs.next()) |dir| {
        const full = std.fs.path.join(r.arena, &.{ dir, name }) catch return false;
        Io.Dir.cwd().access(r.io, full, .{ .execute = true }) catch continue;
        return true;
    }
    return false;
}

fn hasLine(text: []const u8, wanted: []const u8) bool {
    var lines = std.mem.splitScalar(u8, text, '\n');
    while (lines.next()) |line| {
        if (std.mem.eql(u8, line, wanted)) return true;
    }
    return false;
}

fn say(comptime fmt: []const u8, args: anytype) void {
    std.debug.print(fmt ++ "\n", args);
}

fn fail(comptime fmt: []const u8, args: anytype) noreturn {
    std.debug.print("error: " ++ fmt ++ "\n", args);
    std.process.exit(1);
}
