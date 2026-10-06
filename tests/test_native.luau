-- Which renderer binary runs: the package's prebuilt one per OS / CPU, the data-dir build when it
-- matches, adopting a packaged binary that can't start, and no build where none is possible.

local fake = require("./fake_tern")

local function eq(a: any, b: any, what: string)
	if a ~= b then
		error(string.format("%s: expected %s, got %s", what, tostring(b), tostring(a)), 2)
	end
end

local CARGO = '[package]\nname = "office-preview"\nversion = "0.2.0"\n'
local OK = [[{"ok":true,"dir":"/data/cache/k","meta":{"kind":"docx","parts":[{"file":"part-0","label":"1"}]}}]]
local PART = [[{"node":{"k":"el","p":{"tag":"div","class":"opv opd"}},"images":[]}]]

local function setup(opts: any?): any
	local env = fake.new(opts)
	__fresh(env)
	env.files["/data/cache/k/part-0.json"] = PART
	return env
end

local function ok(): any
	return { status = 0, stdout = OK, stderr = "", timed_out = false }
end

-- ── macOS package: the universal binary runs, nothing is built ──────────────────────────────
do
	local env = setup()
	env.files["/plugin/native/Cargo.toml"] = CARGO
	env.files["/plugin/bin/macos/office-preview"] = "MACHO"
	env.files["/plugin/bin/VERSION"] = "0.2.0\n"
	local runs = {}
	env.process = function(argv: { string }, _o: any): any
		table.insert(runs, argv[1])
		return ok()
	end
	require("../host")
	local st = env.blocks.doc.init(env.cx(1), { "/a.docx" }, nil)
	env.drain()
	eq(st.phase, "ready", "rendered with the packaged binary")
	eq(#runs, 1, "no build")
	eq(runs[1], "/plugin/bin/macos/office-preview", "packaged binary used")
end

-- ── a package whose binary is older than its sources is ignored ─────────────────────────────
do
	local env = setup()
	env.files["/plugin/native/Cargo.toml"] = CARGO
	env.files["/plugin/bin/macos/office-preview"] = "MACHO"
	env.files["/plugin/bin/VERSION"] = "0.1.0\n"
	local runs = {}
	env.process = function(argv: { string }, _o: any): any
		table.insert(runs, argv[#argv])
		if argv[1] == "/bin/sh" then
			env.files["/data/bin/office-preview"] = "BUILT"
			env.files["/data/bin/office-preview.version"] = "0.2.0"
			return { status = 0, stdout = "", stderr = "", timed_out = false }
		end
		eq(argv[1], "/data/bin/office-preview", "the fresh build runs")
		return ok()
	end
	require("../host")
	local st = env.blocks.doc.init(env.cx(1), { "/a.docx" }, nil)
	eq(st.phase, "build", "stale package: build from sources")
	env.drain()
	eq(st.phase, "ready", "then render")
end

-- ── Linux: the CPU comes from /bin/sh's ELF header ──────────────────────────────────────────
do
	local env = setup({ os = "linux" })
	env.files["/plugin/bin/linux/office-preview-aarch64"] = "ELF-A64"
	env.files["/plugin/bin/linux/office-preview-x86_64"] = "ELF-X64"
	env.files["/plugin/bin/VERSION"] = "0.2.0"
	-- No native/ shipped: the expected version is bin/VERSION.
	env.files["/bin/sh"] = "\127ELF" .. string.rep("\0", 14) .. "\183\0" .. string.rep("\0", 40)
	local used = nil
	env.process = function(argv: { string }, _o: any): any
		used = argv[1]
		return ok()
	end
	require("../host")
	local st = env.blocks.doc.init(env.cx(1), { "/a.xlsx" }, nil)
	env.drain()
	eq(used, "/plugin/bin/linux/office-preview-aarch64", "aarch64 picked from e_machine 0xB7")
	eq(st.phase, "ready", "rendered")
end

-- ── a packaged binary that can't start is copied into the data dir and retried ──────────────
do
	local env = setup()
	env.files["/plugin/bin/macos/office-preview"] = "MACHO"
	env.files["/plugin/bin/VERSION"] = "0.2.0"
	local calls = {}
	env.process = function(argv: { string }, _o: any): any
		table.insert(calls, table.concat(argv, " "))
		if argv[1] == "/plugin/bin/macos/office-preview" then
			return { status = -1, stdout = "", stderr = "", timed_out = false } -- killed by Gatekeeper
		elseif argv[1] == "chmod" then
			return { status = 0, stdout = "", stderr = "", timed_out = false }
		end
		eq(argv[1], "/data/bin/office-preview", "retried with the adopted copy")
		return ok()
	end
	require("../host")
	local st = env.blocks.doc.init(env.cx(1), { "/a.docx" }, nil)
	env.drain()
	eq(st.phase, "ready", "rendered after adopting")
	eq(env.files["/data/bin/office-preview"], "MACHO", "binary copied")
	eq(env.files["/data/bin/office-preview.version"], "0.2.0\n", "version stamped")
	eq(calls[2], "chmod 755 /data/bin/office-preview", "made executable")
	-- From now on the adopted copy is preferred.
	local again = env.blocks.doc.init(env.cx(1), { "/a.docx" }, nil)
	env.drain()
	eq(string.sub(calls[#calls], 1, 24), "/data/bin/office-preview", "adopted copy runs first")
	eq(again.phase, "ready", "second block rendered")
end

-- ── Windows without a packaged binary: explained, no shell build attempted ──────────────────
do
	local env = setup({ os = "windows" })
	env.files["/plugin/native/Cargo.toml"] = CARGO
	local spawned = 0
	env.process = function(_argv: { string }, _o: any): any
		spawned += 1
		return ok()
	end
	require("../host")
	local st = env.blocks.doc.init(env.cx(1), { "C:\\a.docx" }, nil)
	env.drain()
	eq(spawned, 0, "nothing spawned")
	eq(st.phase, "error", "error shown")
	assert(string.find(st.error, "预编译", 1, true), "explains the missing prebuilt renderer")
end

print("native ok")
