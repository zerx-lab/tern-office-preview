--!strict
-- Office-like selection on a rendered sheet part: an index of the grid's cells by Excel row and
-- column (merged cells cover several), selecting a cell / row / column, moving with the arrows,
-- and marking the view with classes (office.css): `sel` the active cell, `sl` a selected row or
-- column, `hl` the crosshair over the active cell's row and column, `hh` their headers.
--
-- The renderer keys every cell `"<row>:<col>"`, column headers `h<col>` and row headers
-- `r<row>` (1-based Excel numbers) and spans merged cells with `gs<n>` / `gr<n>` classes.

local M = {}

export type Cell = { node: { [string]: any }, r: number, c: number, r2: number, c2: number }
export type Sel = { kind: string, r: number, c: number } -- kind: "cell" | "row" | "col"
export type Index = {
	rows: { number }, -- Excel row numbers in display order
	cols: { number }, -- Excel column numbers in display order
	row_at: { [number]: number }, -- row number → display index
	col_at: { [number]: number },
	cells: { Cell },
	at: { [string]: Cell }, -- "r:c" → the cell covering it
	col_head: { [number]: { [string]: any } },
	row_head: { [number]: { [string]: any } },
	marked: { [{ [string]: any }]: string | false }, -- marked node → its class before marking
}

local function find_grid(node: { [string]: any }): { [string]: any }?
	local p = node.p
	if p and type(p.class) == "string" and string.find(" " .. p.class .. " ", " xg ", 1, true) then
		return node
	end
	for _, c in node.c or {} do
		local g = find_grid(c)
		if g then
			return g
		end
	end
	return nil
end

local function span(class: string?, prefix: string): number
	if class == nil then
		return 1
	end
	local n = string.match(" " .. class .. " ", " " .. prefix .. "(%d+) ")
	return if n then tonumber(n) :: number else 1
end

-- The index of a sheet part, or nil when the part has no grid (a docx part, a chart sheet).
function M.index(root: { [string]: any }): Index?
	local grid = find_grid(root)
	if grid == nil then
		return nil
	end
	local ix: Index = {
		rows = {},
		cols = {},
		row_at = {},
		col_at = {},
		cells = {},
		at = {},
		col_head = {},
		row_head = {},
		marked = {},
	}
	local anchors = {}
	for _, child in grid.c or {} do
		local p = child.p or {}
		local key = p.key
		if type(key) == "string" then
			local r, c = string.match(key, "^(%d+):(%d+)$")
			if r then
				table.insert(anchors, { node = child, r = tonumber(r) :: number, c = tonumber(c) :: number })
			else
				local col = string.match(key, "^h(%d+)$")
				local row = string.match(key, "^r(%d+)$")
				if col then
					local n = tonumber(col) :: number
					table.insert(ix.cols, n)
					ix.col_at[n] = #ix.cols
					ix.col_head[n] = child
				elseif row then
					local n = tonumber(row) :: number
					table.insert(ix.rows, n)
					ix.row_at[n] = #ix.rows
					ix.row_head[n] = child
				end
			end
		end
	end
	for _, a in anchors do
		local ri, ci = ix.row_at[a.r], ix.col_at[a.c]
		if ri and ci then
			local p = a.node.p or {}
			local rs, cs = span(p.class, "gr"), span(p.class, "gs")
			local last_r = ix.rows[math.min(ri + rs - 1, #ix.rows)]
			local last_c = ix.cols[math.min(ci + cs - 1, #ix.cols)]
			local cell: Cell = { node = a.node, r = a.r, c = a.c, r2 = last_r, c2 = last_c }
			table.insert(ix.cells, cell)
			for i = ri, ix.row_at[last_r] do
				for j = ci, ix.col_at[last_c] do
					ix.at[ix.rows[i] .. ":" .. ix.cols[j]] = cell
				end
			end
		end
	end
	return ix
end

-- What a click on node `id` selects: its key after the last dot.
function M.hit(id: string): Sel?
	local key = string.match(id, "([^%.]+)$")
	if key == nil then
		return nil
	end
	local r, c = string.match(key, "^(%d+):(%d+)$")
	if r then
		return { kind = "cell", r = tonumber(r) :: number, c = tonumber(c) :: number }
	end
	local col = string.match(key, "^h(%d+)$")
	if col then
		return { kind = "col", r = 0, c = tonumber(col) :: number }
	end
	local row = string.match(key, "^r(%d+)$")
	if row then
		return { kind = "row", r = tonumber(row) :: number, c = 0 }
	end
	return nil
end

-- The cell covering Excel position (r, c) in this part.
function M.cell(ix: Index, r: number, c: number): Cell?
	return ix.at[r .. ":" .. c]
end

local function add(ix: Index, node: { [string]: any }, class: string)
	local p = node.p or {}
	node.p = p
	if ix.marked[node] == nil then
		ix.marked[node] = p.class or false
	end
	p.class = if p.class and p.class ~= "" then p.class .. " " .. class else class
end

-- Puts back every class `mark` added.
function M.clear(ix: Index)
	for node, class in ix.marked do
		(node.p :: any).class = if class then class else nil
	end
	table.clear(ix.marked)
end

local function overlaps(a1: number, a2: number, b1: number, b2: number): boolean
	return a1 <= b2 and b1 <= a2
end

-- Marks the view for `sel`, with the crosshair over the active cell when `cross` is on.
function M.mark(ix: Index, sel: Sel?, cross: boolean)
	M.clear(ix)
	if sel == nil then
		return
	end
	local row_ix = function(r: number): number
		return ix.row_at[r] or 0
	end
	local col_ix = function(c: number): number
		return ix.col_at[c] or 0
	end
	if sel.kind == "cell" then
		local active = M.cell(ix, sel.r, sel.c)
		if active == nil then
			return
		end
		local r1, r2 = row_ix(active.r), row_ix(active.r2)
		local c1, c2 = col_ix(active.c), col_ix(active.c2)
		add(ix, active.node, "sel")
		for i = r1, r2 do
			local h = ix.row_head[ix.rows[i]]
			if h then
				add(ix, h, "hh")
			end
		end
		for j = c1, c2 do
			local h = ix.col_head[ix.cols[j]]
			if h then
				add(ix, h, "hh")
			end
		end
		if cross then
			for _, cell in ix.cells do
				if cell ~= active then
					local in_rows = overlaps(row_ix(cell.r), row_ix(cell.r2), r1, r2)
					local in_cols = overlaps(col_ix(cell.c), col_ix(cell.c2), c1, c2)
					if in_rows or in_cols then
						add(ix, cell.node, "hl")
					end
				end
			end
		end
	elseif sel.kind == "row" then
		local i = row_ix(sel.r)
		local h = ix.row_head[sel.r]
		if h then
			add(ix, h, "hh")
		end
		for _, cell in ix.cells do
			if overlaps(row_ix(cell.r), row_ix(cell.r2), i, i) then
				add(ix, cell.node, "sl")
			end
		end
	elseif sel.kind == "col" then
		local j = col_ix(sel.c)
		local h = ix.col_head[sel.c]
		if h then
			add(ix, h, "hh")
		end
		for _, cell in ix.cells do
			if overlaps(col_ix(cell.c), col_ix(cell.c2), j, j) then
				add(ix, cell.node, "sl")
			end
		end
	end
end

-- The selection one step from `sel` in direction (dr, dc), skipping over merged cells;
-- `edge` is -1 / 1 when the step leaves the part's first / last row, else 0.
function M.move(ix: Index, sel: Sel?, dr: number, dc: number): (Sel?, number)
	if #ix.rows == 0 or #ix.cols == 0 then
		return sel, 0
	end
	local cur = sel
	if cur == nil or cur.kind ~= "cell" or M.cell(ix, cur.r, cur.c) == nil then
		local r = if cur and cur.kind == "row" and ix.row_at[cur.r] then cur.r else ix.rows[1]
		local c = if cur and cur.kind == "col" and ix.col_at[cur.c] then cur.c else ix.cols[1]
		return { kind = "cell", r = r, c = c }, 0
	end
	local active = M.cell(ix, cur.r, cur.c) :: Cell
	local ri, ci = ix.row_at[cur.r], ix.col_at[cur.c]
	if dr > 0 then
		ri = ix.row_at[active.r2] + 1
	elseif dr < 0 then
		ri = ix.row_at[active.r] - 1
	end
	if dc > 0 then
		ci = ix.col_at[active.c2] + 1
	elseif dc < 0 then
		ci = ix.col_at[active.c] - 1
	end
	if ri < 1 then
		return cur, -1
	elseif ri > #ix.rows then
		return cur, 1
	end
	ci = math.clamp(ci, 1, #ix.cols)
	-- Landing inside a merged cell selects the whole merge (its top-left), as Excel does.
	local target = M.cell(ix, ix.rows[ri], ix.cols[ci])
	if target == nil then
		return cur, 0
	end
	return { kind = "cell", r = target.r, c = target.c }, 0
end

-- "F10" for column 6 row 10.
function M.address(r: number, c: number): string
	local s = ""
	local n = c
	while n > 0 do
		local m = (n - 1) % 26
		s = string.char(65 + m) .. s
		n = (n - 1 - m) // 26
	end
	return s .. tostring(r)
end

-- The text a cell shows (its tooltip when clipped, else its text and runs).
function M.text(cell: Cell): string
	local p = cell.node.p or {}
	if type(p.title) == "string" then
		return p.title
	end
	local out = { if type(p.text) == "string" then p.text else "" }
	local function walk(n: { [string]: any })
		for _, c in n.c or {} do
			local cp = c.p or {}
			if type(cp.text) == "string" then
				table.insert(out, cp.text)
			end
			walk(c)
		end
	end
	walk(cell.node)
	return table.concat(out)
end

return M
