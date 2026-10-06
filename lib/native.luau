--!strict
-- The native renderer (`native/`, Rust): which binary to run, building it when no usable one
-- ships, and running a render. Everything here runs off the worker thread through
-- tern.process.run.
--
-- Binaries, in order of preference:
--   1. `<data>/bin/office-preview` built by scripts/build.sh (or adopted, below), when its
--      `.version` is the expected one;
--   2. the package's prebuilt one (scripts/package.sh): `bin/macos/office-preview` (universal),
--      `bin/linux/office-preview-<x86_64|aarch64>`, `bin/windows/office-preview.exe`, with the
--      crate version in `bin/VERSION`;
--   3. none: build from `native/` (needs cargo; not on Windows).
-- The expected version is native/Cargo.toml's, or bin/VERSION when the sources aren't shipped.

local M = {}

export type Rendered = { dir: string, meta: { [string]: any } }
export type Status = "ready" | "missing" | "stale"

local BUILD_TIMEOUT_MS = 15 * 60 * 1000
local RENDER_TIMEOUT_MS = 5 * 60 * 1000

local function trim(s: string): string
	return (string.match(s, "^%s*(.-)%s*$")) or s
end

local function read_trim(path: string): string?
	local ok, text = pcall(tern.fs.read, path)
	return if ok then trim(text) else nil
end

-- The crate version the plugin's sources declare (native/Cargo.toml).
local function source_version(): string?
	local ok, text = pcall(tern.fs.read, tern.plugin.dir .. "/native/Cargo.toml")
	if not ok then
		return nil
	end
	return string.match(text, '%[package%][^%[]-\nversion%s*=%s*"([^"]+)"')
end

local function expected_version(): string?
	return source_version() or read_trim(tern.plugin.dir .. "/bin/VERSION")
end

-- The CPU of this Linux machine, from the ELF header of /bin/sh (e_machine at byte 18).
local linux_arch_cache: string? = nil
local function linux_arch(): string?
	if linux_arch_cache == nil then
		local ok, head = pcall(tern.fs.read, "/bin/sh")
		if ok and string.sub(head, 1, 4) == "\127ELF" then
			local lo, hi = string.byte(head, 19, 20)
			local machine = lo + hi * 256
			linux_arch_cache = if machine == 0x3E then "x86_64" elseif machine == 0xB7 then "aarch64" else "?"
		else
			linux_arch_cache = "?"
		end
	end
	return if linux_arch_cache == "?" then nil else linux_arch_cache
end

-- The package's prebuilt binary for this machine, when it ships one of the expected version.
function M.packaged(): string?
	local os = tern.runtime.os
	local rel: string? = nil
	if os == "macos" then
		rel = "bin/macos/office-preview"
	elseif os == "windows" then
		rel = "bin/windows/office-preview.exe"
	elseif os == "linux" then
		local arch = linux_arch()
		rel = if arch then "bin/linux/office-preview-" .. arch else nil
	end
	if rel == nil then
		return nil
	end
	local path = tern.plugin.dir .. "/" .. rel
	if not tern.fs.exists(path) then
		return nil
	end
	local want = expected_version()
	if want ~= nil and read_trim(tern.plugin.dir .. "/bin/VERSION") ~= want then
		return nil
	end
	return path
end

-- `<data>/bin/office-preview[.exe]`: built by scripts/build.sh or adopted from the package.
function M.installed(): string
	return tern.plugin.data .. "/bin/office-preview" .. (if tern.runtime.os == "windows" then ".exe" else "")
end

local function installed_ok(): boolean
	local bin = M.installed()
	if not tern.fs.exists(bin) then
		return false
	end
	local want = expected_version()
	return want == nil or read_trim(bin .. ".version") == want
end

-- The binary to run.
function M.binary(): string
	if installed_ok() then
		return M.installed()
	end
	return M.packaged() or M.installed()
end

-- Whether a usable binary exists ("ready"), an old build does ("stale"), or none ("missing").
function M.status(): Status
	if installed_ok() or M.packaged() ~= nil then
		return "ready"
	end
	return if tern.fs.exists(M.installed()) then "stale" else "missing"
end

-- Whether this package can build the renderer itself (sources shipped, a POSIX shell).
function M.can_build(): boolean
	return tern.runtime.os ~= "windows" and tern.fs.exists(tern.plugin.dir .. "/native/Cargo.toml")
end

-- The last non-empty lines of a process's stderr, for an error message.
function M.tail(text: string, lines: number): string
	local out = {}
	for line in string.gmatch(text, "[^\r\n]+") do
		if string.match(line, "%S") then
			table.insert(out, line)
			if #out > lines then
				table.remove(out, 1)
			end
		end
	end
	return table.concat(out, "\n")
end

type Waiter = (ok: boolean, err: string?) -> ()
local waiters: { Waiter }? = nil

function M.building(): boolean
	return waiters ~= nil
end

-- Builds and installs the binary (scripts/build.sh); concurrent callers share one build.
function M.build(done: Waiter)
	local pending = waiters
	if pending then
		table.insert(pending, done)
		return
	end
	local list: { Waiter } = { done }
	waiters = list
	local function finish(ok: boolean, err: string?)
		waiters = nil
		for _, w in list do
			w(ok, err)
		end
	end
	local spawned, spawn_err = pcall(
		tern.process.run,
		{ "/bin/sh", tern.plugin.dir .. "/scripts/build.sh" },
		{ timeout_ms = BUILD_TIMEOUT_MS },
		function(r: ProcessResult)
			if r.timed_out then
				finish(false, "构建超时")
			elseif r.status ~= 0 or not tern.fs.exists(M.installed()) then
				local msg = M.tail(r.stderr, 6)
				finish(false, if msg ~= "" then msg else "构建失败（退出码 " .. tostring(r.status) .. "）")
			else
				finish(true, nil)
			end
		end
	)
	if not spawned then
		finish(false, tostring(spawn_err))
	end
end

-- Copies the package's binary into `<data>/bin` and marks it executable. For a packaged binary
-- that can't start where it is: an archive that lost the executable bit, or a macOS download
-- whose quarantine flag Gatekeeper refuses (a copy Tern writes carries no such flag).
local function adopt(packaged: string, done: (ok: boolean) -> ())
	local bin = M.installed()
	local ok = pcall(function()
		tern.fs.mkdir(tern.plugin.data .. "/bin")
		tern.fs.write(bin, tern.fs.read(packaged))
		tern.fs.write(bin .. ".version", (expected_version() or "") .. "\n")
	end)
	if not ok then
		done(false)
	elseif tern.runtime.os == "windows" then
		done(true)
	elseif not pcall(tern.process.run, { "chmod", "755", bin }, function(r: ProcessResult)
		done(r.status == 0)
	end) then
		done(false)
	end
end

-- Runs one render with `bin`; `broken` says the binary itself could not run.
local function run(bin: string, path: string, cb: (res: Rendered?, err: string?, broken: boolean) -> ())
	local argv = { bin, "render", path, "--cache", tern.plugin.data .. "/cache" }
	local spawned, spawn_err = pcall(tern.process.run, argv, { timeout_ms = RENDER_TIMEOUT_MS }, function(r: ProcessResult)
		if r.timed_out then
			cb(nil, "渲染超时（超过 5 分钟）", false)
			return
		end
		local ok, out = pcall(tern.json.decode, r.stdout)
		if ok and type(out) == "table" then
			if out.ok == true and type(out.dir) == "string" and type(out.meta) == "table" then
				cb({ dir = out.dir, meta = out.meta }, nil, false)
			else
				cb(nil, if type(out.error) == "string" then out.error else "渲染失败", false)
			end
			return
		end
		local msg = M.tail(r.stderr, 4)
		-- 126/127: not executable / not found; -1 with no output: killed at launch (Gatekeeper).
		local broken = r.status == 126 or r.status == 127 or (r.status == -1 and r.stdout == "")
		cb(nil, if msg ~= "" then msg else "渲染器异常退出（" .. tostring(r.status) .. "）", broken)
	end)
	if not spawned then
		cb(nil, tostring(spawn_err), true)
	end
end

-- Renders `path` into `<data>/cache` and calls `cb` with the cache entry or an error. A packaged
-- binary that can't start is adopted into `<data>/bin` and tried once more.
function M.render(path: string, cb: (res: Rendered?, err: string?) -> ())
	local bin = M.binary()
	run(bin, path, function(res, err, broken)
		if not broken or bin ~= M.packaged() then
			cb(res, err)
			return
		end
		adopt(bin, function(ok)
			if not ok then
				cb(nil, err)
				return
			end
			run(M.installed(), path, function(res2, err2)
				cb(res2, err2)
			end)
		end)
	end)
end

-- A part file: `{node, images = {{file, mime}}}`.
function M.read_part(dir: string, file: string): ({ [string]: any }, { { file: string, mime: string } })
	local v = tern.json.decode(tern.fs.read(dir .. "/" .. file .. ".json"))
	return v.node, v.images or {}
end

return M
