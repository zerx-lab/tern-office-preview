-- Sheet selection: the cell index over merged cells, clicks, marks with and without the
-- crosshair, arrow moves across merges and part edges, and the switch kept in tern.kv.

local fake = require("./fake_tern")

local function eq(a: any, b: any, what: string)
	if a ~= b then
		error(string.format("%s: expected %s, got %s", what, tostring(b), tostring(a)), 2)
	end
end

local function has(node: any, class: string): boolean
	return string.find(" " .. tostring(node.p.class) .. " ", " " .. class .. " ", 1, true) ~= nil
end

-- A sheet part as the renderer writes it: rows `first`..`first+n-1`, columns A–C, with
-- A<first>:C<first> merged and B<first+1>:B<first+2> merged.
local function part(first: number, n: number): string
	local cells = { '{"k":"el","p":{"tag":"div","class":"xc","key":"x"}}' }
	for c = 1, 3 do
		table.insert(cells, string.format('{"k":"el","p":{"tag":"div","class":"xh w16","key":"h%d","text":"%s"}}', c, string.char(64 + c)))
	end
	for r = first, first + n - 1 do
		table.insert(cells, string.format('{"k":"el","p":{"tag":"div","class":"xr","key":"r%d","text":"%d"}}', r, r))
		local function cell(c: number, class: string, text: string)
			table.insert(cells, string.format('{"k":"el","p":{"tag":"div","class":"%s","key":"%d:%d","text":"%s","actions":{"click":"c"}}}', class, r, c, text))
		end
		if r == first then
			cell(1, "w48 gs3", "标题")
		elseif r == first + 1 then
			cell(1, "w16", "a")
			cell(2, "w16 gr2", "merged")
			cell(3, "w16", "c")
		elseif r == first + 2 then
			cell(1, "w16", "a")
			cell(3, "w16", "c")
		else
			for c = 1, 3 do
				cell(c, "w16", string.char(96 + c) .. r)
			end
		end
	end
	return '{"node":{"k":"el","p":{"tag":"div","class":"opv opx"},"c":[{"k":"el","p":{"tag":"div","class":"xg gt4"},"c":['
		.. table.concat(cells, ",")
		.. "]}]},\"images\":[]}"
end

-- ── the index and marks, directly ───────────────────────────────────────────────────────────
do
	local env = fake.new()
	__fresh(env)
	local grid = require("../lib/grid")
	local root = env.tern.json.decode(part(1, 5)).node
	local ix = grid.index(root)
	assert(ix, "sheet grid indexed")
	eq(#ix.rows, 5, "rows")
	eq(#ix.cols, 3, "cols")
	eq(grid.cell(ix, 1, 3).c, 1, "C1 is covered by A1:C1")
	eq(grid.cell(ix, 3, 2).r, 2, "B3 is covered by B2:B3")
	eq(grid.cell(ix, 3, 2).r2, 3, "the merge ends at row 3")
	eq(grid.address(10, 6), "F10", "address")
	eq(grid.address(3, 28), "AB3", "two-letter column")

	local hit = grid.hit("main.s1-s0-0.0.2:2") :: any
	eq(hit.kind, "cell", "cell click")
	eq(hit.r * 10 + hit.c, 22, "cell click position")
	eq((grid.hit("main.x.0.h3") :: any).kind, "col", "column header click")
	eq((grid.hit("main.x.0.r4") :: any).r, 4, "row header click")

	local b2 = grid.cell(ix, 2, 2)
	grid.mark(ix, { kind = "cell", r = 2, c = 2 }, false)
	assert(has(b2.node, "sel"), "active cell outlined")
	assert(has(ix.row_head[2], "hh") and has(ix.row_head[3], "hh"), "headers of both merged rows")
	assert(has(ix.col_head[2], "hh"), "column header")
	assert(not has(grid.cell(ix, 2, 1).node, "hl"), "no crosshair when off")

	grid.mark(ix, { kind = "cell", r = 2, c = 2 }, true)
	assert(not has(b2.node, "hl"), "active cell isn't tinted")
	assert(has(grid.cell(ix, 2, 1).node, "hl"), "same row tinted")
	assert(has(grid.cell(ix, 3, 3).node, "hl"), "row 3 is part of the merge's rows")
	assert(has(grid.cell(ix, 1, 1).node, "hl"), "the A1:C1 merge crosses column B")
	assert(has(grid.cell(ix, 5, 2).node, "hl"), "same column tinted")
	assert(not has(grid.cell(ix, 4, 1).node, "hl"), "other cells untouched")
	eq(grid.cell(ix, 2, 1).node.p.class, "w16 hl", "one mark per class, appended once")

	grid.mark(ix, { kind = "col", r = 0, c = 3 }, true)
	assert(not has(b2.node, "sel"), "previous marks cleared")
	eq(b2.node.p.class, "w16 gr2", "original class restored")
	assert(has(grid.cell(ix, 4, 3).node, "sl"), "column selected")
	assert(has(grid.cell(ix, 1, 1).node, "sl"), "merge reaching the column selected")
	grid.mark(ix, nil, true)
	eq(next(ix.marked), nil, "clear leaves nothing marked")

	-- Arrows: into a merge selects its top-left; out of it continues past its far edge.
	local s = grid.move(ix, { kind = "cell", r = 2, c = 1 }, 0, 1) :: any
	eq(s.r * 10 + s.c, 22, "right into B2")
	s = grid.move(ix, s, 1, 0) :: any
	eq(s.r * 10 + s.c, 42, "down from B2:B3 lands on B4")
	s = grid.move(ix, s, -1, 0) :: any
	eq(s.r * 10 + s.c, 22, "up from B4 selects the merge")
	s = grid.move(ix, s, -1, 0) :: any
	eq(s.r * 10 + s.c, 11, "up into A1:C1")
	local _, edge = grid.move(ix, s, -1, 0)
	eq(edge, -1, "up from the first row leaves the part")
	local _, edge2 = grid.move(ix, { kind = "cell", r = 5, c = 3 }, 1, 0)
	eq(edge2, 1, "down from the last row leaves the part")
	local first = grid.move(ix, nil, 1, 0) :: any
	eq(first.r * 10 + first.c, 11, "no selection: arrows start at the first cell")
	eq(grid.text(grid.cell(ix, 1, 2)), "标题", "text of a merged cell")
end

-- ── the block: click, crosshair switch kept in kv, arrows across parts, copy ───────────────
do
	local env = fake.new()
	__fresh(env)
	local function grid_cell(st: any, r: number, c: number): any
		return require("../lib/grid").cell(st.grid, r, c).node
	end
	env.files["/plugin/native/Cargo.toml"] = '[package]\nname = "x"\nversion = "0.1.0"\n'
	env.files["/data/bin/office-preview"] = "bin"
	env.files["/data/bin/office-preview.version"] = "0.1.0"
	env.files["/data/cache/k/s0-0.json"] = part(1, 5)
	env.files["/data/cache/k/s0-1.json"] = part(6, 5)
	env.process = function(_argv: { string }, _opts: any): any
		return {
			status = 0,
			stdout = [[{"ok":true,"dir":"/data/cache/k","meta":{"kind":"xlsx","active":0,"sheets":[{"name":"S","state":"visible","rows":10,"cols":3,"parts":[{"file":"s0-0","label":"1–5"},{"file":"s0-1","label":"6–10"}]}]}}]],
			stderr = "",
			timed_out = false,
		}
	end
	require("../host")
	local def = env.blocks.doc
	local cx = env.cx(1)
	local st = def.init(cx, { "/x.xlsx" }, nil)
	env.drain()
	eq(st.phase, "ready", "rendered")
	eq(st.cross, false, "crosshair off by default")

	def.event(st, { ev = "action", id = "main.s1-s0-0.0.2:2", act = "c" }, cx)
	eq(st.sel.r * 10 + st.sel.c, 22, "click selects")
	local v = def.view(st, cx)
	local info = nil
	for _, n in v.dock.c[1].c do
		if n.p and n.p.key == "cell" then
			info = n.p.text
		end
	end
	eq(info, "B2  merged", "name box shows address and text")

	eq(def.key(st, { name = "h", text = "h" }, cx), true, "h toggles the crosshair")
	eq(env.kv.crosshair, true, "switch remembered in kv")
	assert(has(grid_cell(st, 2, 1), "hl"), "crosshair drawn at once")

	-- A new block reads the switch back.
	local st2 = def.init(env.cx(1), { "/x.xlsx" }, nil)
	eq(st2.cross, true, "new blocks start with the remembered switch")

	-- Down past the last row continues in the next part, same column.
	def.event(st, { ev = "action", id = "main.s1-s0-0.0.5:2", act = "c" }, cx)
	eq(def.key(st, { name = "down" }, cx), true, "down at the edge")
	eq(st.part, 2, "next part")
	eq(st.sel.r * 10 + st.sel.c, 61, "B6 is inside the A6:C6 merge, which gets selected")
	eq(def.key(st, { name = "up" }, cx), true, "up at the top edge")
	eq(st.part, 1, "back to the first part")
	eq(st.sel.r, 5, "last row of the previous part")

	def.key(st, { name = "y", text = "y" }, cx)
	eq(cx.copied, "a5", "y copies the cell text")
	eq(def.key(st, { name = "escape" }, cx), true, "escape clears")
	eq(st.sel, nil, "no selection")
	eq(def.key(st, { name = "escape" }, cx), false, "escape with nothing selected does nothing")
end

print("grid ok")
