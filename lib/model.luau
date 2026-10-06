--!strict
-- Pure helpers over a rendered document's meta: which parts and sheets there are, moving
-- between them, zoom steps, and swapping image `src`s for blob ids.

local M = {}

export type Part = { file: string, label: string }
export type Sheet = { name: string, state: string?, rows: number?, cols: number?, parts: { Part }, notes: { string }? }
export type Meta = {
	kind: string,
	name: string?,
	title: string?,
	parts: { Part }?,
	outline: { { text: string, level: number, part: number } }?,
	sheets: { Sheet }?,
	active: number?,
	notes: { string }?,
}

-- Zoom levels in percent; office.css has a `z<n>` class for each but 100.
M.ZOOMS = { 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200 }

-- 1-based indices of the sheets shown as tabs (very hidden sheets never are).
function M.tabs(meta: Meta): { number }
	local out = {}
	for i, s in meta.sheets or {} do
		if s.state ~= "veryHidden" then
			table.insert(out, i)
		end
	end
	return out
end

-- The sheet to open first: the workbook's active tab when shown, else the first tab.
function M.first_sheet(meta: Meta): number
	local tabs = M.tabs(meta)
	local active = (meta.active or 0) + 1
	for _, i in tabs do
		if i == active then
			return i
		end
	end
	return tabs[1] or 1
end

-- Parts of what is on screen: the document's, or the current sheet's.
function M.parts(meta: Meta, sheet: number): { Part }
	if meta.kind == "xlsx" then
		local s = (meta.sheets or {})[sheet]
		return if s then s.parts else {}
	end
	return meta.parts or {}
end

function M.clamp(n: number, lo: number, hi: number): number
	if hi < lo then
		return lo
	end
	return math.max(lo, math.min(hi, n))
end

-- The sheet `delta` tabs away from `sheet` (wrapping), or `sheet` when there is no other.
function M.step_sheet(meta: Meta, sheet: number, delta: number): number
	local tabs = M.tabs(meta)
	if #tabs == 0 then
		return sheet
	end
	local at = 1
	for i, s in tabs do
		if s == sheet then
			at = i
		end
	end
	return tabs[((at - 1 + delta) % #tabs) + 1]
end

-- The next zoom level in direction `dir` (+1 / -1) from `z`.
function M.zoom_step(z: number, dir: number): number
	local levels = M.ZOOMS
	if dir > 0 then
		for _, v in levels do
			if v > z then
				return v
			end
		end
		return levels[#levels]
	end
	for i = #levels, 1, -1 do
		if levels[i] < z then
			return levels[i]
		end
	end
	return levels[1]
end

-- Replaces every `image` node's `src` with its blob id from `blobs` (by file), in place.
function M.attach(node: { [string]: any }, blobs: { [string]: string })
	if node.k == "image" and node.p and node.p.src then
		local id = blobs[node.p.src]
		node.p.src = nil
		node.p.blob = id
	end
	local children = node.c
	if children then
		for _, c in children do
			M.attach(c, blobs)
		end
	end
end

-- "report.docx" from "/a/b/report.docx".
function M.basename(path: string): string
	return string.match(path, "([^/\\]+)$") or path
end

return M
