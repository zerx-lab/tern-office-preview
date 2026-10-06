-- Host and window halves against a scripted runtime: first-use build, render, part and sheet
-- navigation, blobs, zoom, outline, errors, restore from `saved`, and file routing.

local fake = require("./fake_tern")

local function eq(a: any, b: any, what: string)
	if a ~= b then
		error(string.format("%s: expected %s, got %s", what, tostring(b), tostring(a)), 2)
	end
end

-- Visible text of a view tree (span texts, `text` props, tab labels).
local function walk(node: any, out: { string })
	assert(type(node) == "table" and type(node.k) == "string", "bad node")
	local p = node.p or {}
	for _, key in { "spans", "head", "label" } do
		local s = p[key]
		if type(s) == "table" then
			for _, span in s do
				table.insert(out, if type(span) == "table" then span.t else span)
			end
		elseif type(s) == "string" then
			table.insert(out, s)
		end
	end
	if type(p.text) == "string" then
		table.insert(out, p.text)
	end
	for _, item in p.items or {} do
		table.insert(out, tostring(item.label))
	end
	for _, c in node.c or {} do
		walk(c, out)
	end
end

local function text(v: any): string
	local out = {}
	walk(v.main, out)
	if v.dock then
		walk(v.dock, out)
	end
	return table.concat(out, " | ")
end

-- First node (depth-first) for which `pred` holds.
local function find(node: any, pred: (any) -> boolean): any
	if pred(node) then
		return node
	end
	for _, c in node.c or {} do
		local hit = find(c, pred)
		if hit then
			return hit
		end
	end
	return nil
end

local CARGO = '[package]\nname = "office-preview"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nzip = { version = "2" }\n'
local BIN = "/data/bin/office-preview"
local DIR = "/data/cache/abc"

local function part(class: string, children: string): string
	return string.format('{"node":{"k":"el","p":{"tag":"div","class":"%s"},"c":[%s]},"images":[]}', class, children)
end

local function installed(env: any, version: string)
	env.files[BIN] = "binary"
	env.files[BIN .. ".version"] = version .. "\n"
end

-- A process script: build.sh installs the binary; the renderer answers `answer()`.
local function script(env: any, answer: () -> string): { build: number, render: number }
	local counts = { build = 0, render = 0 }
	env.process = function(argv: { string }, _opts: any): any
		if argv[1] == "/bin/sh" and string.find(argv[2], "scripts/build.sh", 1, true) then
			counts.build += 1
			installed(env, "0.1.0")
			return { status = 0, stdout = BIN .. "\n", stderr = "", timed_out = false }
		elseif argv[1] == BIN then
			counts.render += 1
			eq(argv[2], "render", "render verb")
			eq(argv[4], "--cache", "cache flag")
			eq(argv[5], "/data/cache", "cache dir")
			return { status = 0, stdout = answer(), stderr = "", timed_out = false }
		end
		return { status = 127, stdout = "", stderr = "unexpected " .. argv[1], timed_out = false }
	end
	return counts
end

local DOCX_META = [[{"ok":true,"dir":"/data/cache/abc","meta":{"kind":"docx","name":"r.docx","parts":[{"file":"part-0","label":"1"},{"file":"part-1","label":"2"}],"outline":[{"text":"引言","level":1,"part":0},{"text":"结论","level":2,"part":1}],"notes":["含 1 个无法显示的 EMF 图片"]}}]]

-- ── docx: first use builds the renderer, then render, parts, blobs, zoom, outline ──────────
do
	local env = fake.new()
	__fresh(env)
	env.files["/plugin/native/Cargo.toml"] = CARGO
	env.files[DIR .. "/part-0.json"] = [[{"node":{"k":"el","p":{"tag":"div","class":"opv opd"},"c":[{"k":"el","p":{"tag":"p","text":"第一部分"}},{"k":"image","p":{"src":"img/1.png","w":10,"h":5,"alt":"图"}}]},"images":[{"file":"img/1.png","mime":"image/png"}]}]]
	env.files[DIR .. "/part-1.json"] = [[{"node":{"k":"el","p":{"tag":"div","class":"opv opd"},"c":[{"k":"image","p":{"src":"img/1.png"}}]},"images":[{"file":"img/1.png","mime":"image/png"}]}]]
	env.files[DIR .. "/img/1.png"] = "PNG-BYTES"
	local counts = script(env, function()
		return DOCX_META
	end)
	require("../host")
	local def = env.blocks.doc
	local cx = env.cx(1)
	local st = def.init(cx, { "/docs/r.docx" }, nil)
	eq(st.phase, "build", "missing binary builds first")
	assert(string.find(text(def.view(st, cx)), "正在编译", 1, true), "build screen")
	env.drain()
	eq(counts.build, 1, "one build")
	eq(counts.render, 1, "then one render")
	eq(st.phase, "ready", "ready after render")
	eq(def.title(st), "r.docx", "title is the file name")

	local v = def.view(st, cx)
	local root = v.main.c[1]
	eq(root.p.key, "s1-part-0", "part keyed by sheet and file")
	eq(root.p.class, "opv opd", "root class kept")
	local img = find(root, function(n)
		return n.k == "image"
	end)
	eq(img.p.blob, "blob-1", "image src swapped for the blob id")
	eq(img.p.src, nil, "src removed")
	eq(#env.blobs, 1, "one blob sent")
	eq(env.blobs[1].mime, "image/png", "blob mime")
	eq(env.blobs[1].bytes, "PNG-BYTES", "blob bytes")
	local t = text(v)
	assert(string.find(t, "第 1 部分 / 共 2 部分", 1, true), "pager: " .. t)
	assert(string.find(t, "1 条提示", 1, true), "notes badge")

	eq(def.key(st, { name = "]", text = "]" }, cx), true, "] moves on")
	eq(st.part, 2, "second part")
	eq(def.key(st, { name = "]", text = "]" }, cx), false, "no part past the last")
	eq(#env.blobs, 1, "known picture not sent again")
	eq(def.view(st, cx).main.c[1].p.key, "s1-part-1", "second part shown")

	def.key(st, { name = "=", text = "+", shift = true }, cx)
	eq(st.zoom, 110, "+ zooms in")
	eq(def.view(st, cx).main.c[1].p.class, "opv opd z110", "zoom class on the root")
	def.key(st, { name = "0", text = "0" }, cx)
	eq(def.view(st, cx).main.c[1].p.class, "opv opd", "0 resets zoom")

	def.key(st, { name = "o", text = "o" }, cx)
	v = def.view(st, cx)
	local split = v.main.c[1]
	eq(split.p.class, "op-split", "outline beside the document")
	local current = find(split, function(n)
		return n.p and type(n.p.class) == "string" and string.find(n.p.class, " on", 1, true) ~= nil
	end)
	eq(current.p.text, "结论", "heading of the current part is marked")
	def.event(st, { ev = "action", act = "goto", value = "1" }, cx)
	eq(st.part, 1, "outline click jumps")

	local saved = def.save(st)
	eq(saved.path, "/docs/r.docx", "saved path")
	eq(saved.part, 1, "saved part")
	eq(saved.outline, true, "saved outline")

	def.key(st, { name = "r", text = "r" }, cx)
	eq(st.phase, "ready", "reload keeps the old part up")
	env.drain()
	eq(counts.render, 2, "reload renders again")
	eq(st.part, 1, "reload keeps the part")
end

-- ── xlsx: tabs skip very hidden sheets, active sheet, sheet keys, restore ────────────────────
local XLSX_META = [[{"ok":true,"dir":"/data/cache/abc","meta":{"kind":"xlsx","active":2,"sheets":[
 {"name":"数据","state":"visible","rows":500,"cols":3,"parts":[{"file":"s0-0","label":"1–200"},{"file":"s0-1","label":"201–400"},{"file":"s0-2","label":"401–500"}]},
 {"name":"秘密","state":"veryHidden","rows":1,"cols":1,"parts":[{"file":"s1-0","label":"1–1"}]},
 {"name":"备注","state":"hidden","rows":2,"cols":2,"parts":[{"file":"s2-0","label":"1–2"}],"notes":["含条件格式（未显示）"]}]}}]]

do
	local env = fake.new()
	__fresh(env)
	env.files["/plugin/native/Cargo.toml"] = CARGO
	installed(env, "0.1.0")
	for _, f in { "s0-0", "s0-1", "s0-2", "s1-0", "s2-0" } do
		env.files[DIR .. "/" .. f .. ".json"] = part("opv opx", '{"k":"el","p":{"tag":"p","text":"' .. f .. '"}}')
	end
	local counts = script(env, function()
		return XLSX_META
	end)
	require("../host")
	local def = env.blocks.doc
	local cx = env.cx(1)
	local st = def.init(cx, { "/docs/b.xlsx" }, nil)
	eq(st.phase, "render", "installed binary renders at once")
	env.drain()
	eq(counts.build, 0, "no build")
	eq(st.sheet, 3, "opens the workbook's active sheet")
	local v = def.view(st, cx)
	local tabs = find(v.dock, function(n)
		return n.k == "tabs"
	end)
	eq(#tabs.p.items, 2, "very hidden sheet has no tab")
	eq(tabs.p.items[2].label, "备注（隐藏）", "hidden sheet is labeled")
	eq(tabs.p.active, "3", "active tab")
	assert(string.find(text(v), "1 条提示", 1, true), "sheet notes counted")

	eq(def.key(st, { name = "]", text = "}", shift = true }, cx), true, "} next sheet")
	eq(st.sheet, 1, "wraps to the first tab")
	eq(def.key(st, { name = "]", text = "]" }, cx), true, "] next rows")
	eq(st.part, 2, "rows 201–400")
	assert(string.find(text(def.view(st, cx)), "行 201–400 / 共 3 部分", 1, true), "row pager")
	def.key(st, { name = "g", text = "G", shift = true }, cx)
	eq(st.part, 3, "G goes to the last part")
	def.event(st, { ev = "select", id = "dock.0.tabs", item = "3" }, cx)
	eq(st.sheet, 3, "tab click switches sheet")
	eq(st.part, 1, "new sheet starts at its first part")
	def.key(st, { name = "tab", shift = true }, cx)
	eq(st.sheet, 1, "shift+tab previous sheet")
	eq(def.key(st, { name = "x", text = "x" }, cx), false, "unused key renders nothing")
	eq(def.key(st, { name = "]", text = "]", meta = true }, cx), false, "cmd chords are not ours")

	-- Restore after a reload / daemon restart: saved state wins over args.
	local cx2 = env.cx(1)
	local st2 = def.init(cx2, { "/other.xlsx" }, { path = "/docs/b.xlsx", sheet = 1, part = 2, zoom = 125 })
	env.drain()
	eq(st2.path, "/docs/b.xlsx", "saved path")
	eq(st2.sheet, 1, "saved sheet")
	eq(st2.part, 2, "saved part")
	eq(def.view(st2, cx2).main.c[1].p.class, "opv opx z125", "saved zoom")
end

-- ── errors: renderer refusal, retry, stale binary rebuild, no argument ──────────────────────
do
	local env = fake.new()
	__fresh(env)
	env.files["/plugin/native/Cargo.toml"] = CARGO
	installed(env, "0.0.9")
	local answer = [[{"ok":false,"error":"这是旧版二进制格式（.doc / .xls）"}]]
	local counts = script(env, function()
		return answer
	end)
	require("../host")
	local def = env.blocks.doc
	local cx = env.cx(1)
	local st = def.init(cx, { "/docs/old.doc.docx" }, nil)
	eq(st.phase, "build", "stale binary is rebuilt")
	env.drain()
	eq(counts.build, 1, "rebuilt once")
	eq(st.phase, "error", "renderer error shown")
	assert(string.find(text(def.view(st, cx)), "旧版二进制格式", 1, true), "error text")
	answer = DOCX_META
	env.files[DIR .. "/part-0.json"] = part("opv opd", "")
	def.key(st, { name = "r", text = "r" }, cx)
	env.drain()
	eq(counts.render, 2, "r retries")
	eq(st.phase, "ready", "retry succeeds")

	local st3 = def.init(env.cx(1), {}, nil)
	eq(st3.phase, "error", "no file is an error")
	assert(string.find(text(def.view(st3, cx)), "没有指定文件", 1, true), "explains how to open")
end

-- ── a failed build is shown with cargo's last lines ─────────────────────────────────────────
do
	local env = fake.new()
	__fresh(env)
	env.files["/plugin/native/Cargo.toml"] = CARGO
	env.process = function(_argv: { string }, _opts: any): any
		return { status = 127, stdout = "", stderr = "\n未找到 cargo：请先安装 Rust 工具链\n", timed_out = false }
	end
	require("../host")
	local def = env.blocks.doc
	local cx = env.cx(1)
	local st = def.init(cx, { "/docs/a.xlsx" }, nil)
	env.drain()
	eq(st.phase, "error", "build failure")
	assert(string.find(st.error, "未找到 cargo", 1, true), "cargo message: " .. tostring(st.error))
	-- A block whose pane went away ignores late results.
	env.panes = {}
	def.key(st, { name = "r", text = "r" }, cx)
	env.drain()
	eq(st.phase, "build", "late build result ignored for a closed pane")
end

-- ── window: routes Office files to the block ────────────────────────────────────────────────
do
	local env = fake.new()
	__fresh(env)
	require("../window")
	local route = env.routes[1]
	local function open(path: string): any
		return route({ path = path, host = "local", how = "default", origin = "files" }, {})
	end
	local d = open("/x/Report.DOCX")
	eq(d.block, "office-preview.doc", "docx routed")
	eq(d.args[1], "/x/Report.DOCX", "path passed")
	eq(open("/x/b.xlsm").block, "office-preview.doc", "xlsm routed")
	eq(open("/x/~$Report.docx"), nil, "Office lock file left alone")
	eq(open("/x/notes.txt"), nil, "other files left alone")
	eq(open("/x/docx"), nil, "no extension")
end

print("block flows ok")
