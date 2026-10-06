--!strict
-- The block's view: the rendered part in `main`, sheet tabs / pager / zoom in `dock`, and the
-- waiting and error screens.

local grid = require("./grid")
local model = require("./model")

local M = {}

export type State = {
	path: string,
	name: string,
	phase: string, -- "build" | "render" | "ready" | "error"
	error: string?,
	dir: string?,
	meta: model.Meta?,
	sheet: number,
	part: number,
	zoom: number,
	outline: boolean,
	node: { [string]: any }?,
	node_class: string,
	node_key: string,
	blobs: { [string]: string },
	gen: number,
	grid: grid.Index?, -- sheet parts: the cells by row / column, for selection
	sel: grid.Sel?,
	cross: boolean, -- crosshair over the active cell's row and column
}

local ui = tern.ui

local function btn(text: string, act: string, title: string, disabled: boolean?): Node
	return ui.el("button", {
		class = if disabled then "op-btn off" else "op-btn",
		text = text,
		title = title,
		actions = if disabled then nil else { click = act },
	})
end

local function muted(text: string): Node
	return ui.text({ ui.span(text, "muted") })
end

local function waiting(label: string, detail: string): { [string]: Node }
	return {
		main = ui.col({
			ui.el("div", { class = "op-wait" }, {
				ui.node("spinner", { style = "dots", label = { ui.span(label) } }),
				muted(detail),
			}),
		}),
	}
end

local function failed(state: State): { [string]: Node }
	local lines = {}
	for line in string.gmatch(state.error or "未知错误", "[^\n]+") do
		table.insert(lines, ui.text({ ui.span(line) }))
	end
	local card = ui.card({ ui.span("无法预览 " .. state.name, "error") }, {
		ui.lines(lines),
		muted("r 重试 · b 重新构建渲染器"),
	})
	card.p = card.p or {}
	;(card.p :: any).tone = "error"
	return { main = ui.col({ ui.el("div", { class = "op-wait" }, { card }) }) }
end

local function outline_panel(meta: model.Meta, part: number): Node
	local items = {}
	for i, h in meta.outline or {} do
		local level = math.clamp(h.level or 1, 1, 6)
		local current = (h.part + 1) == part
		table.insert(
			items,
			ui.el("div", {
				key = "o" .. i,
				class = "op-oi op-l" .. level .. (if current then " on" else ""),
				text = h.text,
				title = h.text,
				actions = { click = "goto=" .. tostring(h.part + 1) },
			})
		)
	end
	if #items == 0 then
		table.insert(items, ui.el("div", { class = "op-oi op-l1", text = "（没有标题）" }))
	end
	return ui.el("div", { class = "op-ol", key = "outline" }, items)
end

local function document(state: State): Node
	local node = state.node :: { [string]: any }
	local p = node.p or {}
	node.p = p
	p.class = state.node_class .. (if state.zoom ~= 100 then " z" .. tostring(state.zoom) else "")
	p.key = state.node_key
	return node :: any
end

local function dock(state: State, meta: model.Meta): Node
	local parts = model.parts(meta, state.sheet)
	local children: { Node } = {}

	if meta.kind == "xlsx" then
		local items = {}
		for _, i in model.tabs(meta) do
			local s = (meta.sheets :: { model.Sheet })[i]
			local label = s.name
			if s.state == "hidden" then
				label ..= "（隐藏）"
			end
			table.insert(items, { id = tostring(i), label = label })
		end
		table.insert(children, ui.node("tabs", { key = "tabs", items = items, active = tostring(state.sheet), grow = 1 }))
		-- The name box and formula bar, in one line: "F10  必选".
		local ix, sel = state.grid, state.sel
		local info = ""
		if ix and sel then
			if sel.kind == "cell" then
				local cell = grid.cell(ix, sel.r, sel.c)
				if cell then
					info = grid.address(cell.r, cell.c) .. "  " .. string.gsub(grid.text(cell), "%s+", " ")
				end
			elseif sel.kind == "row" then
				info = "第 " .. sel.r .. " 行"
			else
				info = grid.address(1, sel.c):gsub("%d+$", "") .. " 列"
			end
		end
		table.insert(children, ui.el("span", { key = "cell", class = "op-cell", text = info, title = info }))
	else
		table.insert(children, ui.el("div", { key = "fill", class = "op-fill" }))
	end

	local n = #parts
	if n > 1 then
		local label = (parts[state.part] or { label = "" }).label
		local what = if meta.kind == "xlsx" then "行 " .. label else "第 " .. label .. " 部分"
		table.insert(children, btn("‹", "prev", "上一部分（[ 或 p）", state.part <= 1))
		table.insert(children, ui.el("span", { key = "pos", class = "op-pos", text = what .. " / 共 " .. n .. " 部分" }))
		table.insert(children, btn("›", "next", "下一部分（] 或 n）", state.part >= n))
	end

	local notes = {}
	for _, s in meta.notes or {} do
		table.insert(notes, s)
	end
	if meta.kind == "xlsx" then
		local s = (meta.sheets or {})[state.sheet]
		for _, note in (s and s.notes) or {} do
			table.insert(notes, note)
		end
	end
	if #notes > 0 then
		local b = ui.badge(#notes .. " 条提示", "warning")
		b.p = b.p or {}
		;(b.p :: any).title = table.concat(notes, "\n")
		;(b.p :: any).key = "notes"
		table.insert(children, b)
	end

	if meta.kind == "docx" then
		table.insert(children, btn("目录", "outline", "显示 / 隐藏目录（o）"))
	elseif state.grid then
		table.insert(children, ui.el("button", {
			key = "cross",
			class = if state.cross then "op-btn on" else "op-btn",
			text = "十字高亮",
			title = "高亮选中单元格所在的行和列（h），开关会被记住",
			actions = { click = "cross" },
		}))
	end
	table.insert(children, btn("−", "zoom=-1", "缩小（-）"))
	table.insert(children, ui.el("button", {
		key = "zoom",
		class = "op-btn op-zoom",
		text = tostring(state.zoom) .. "%",
		title = "恢复 100%（0）",
		actions = { click = "zoom=0" },
	}))
	table.insert(children, btn("+", "zoom=1", "放大（+）"))
	table.insert(children, btn("↻", "reload", "重新加载（r）"))

	return ui.col({ ui.row(children) })
end

function M.render(state: State): { [string]: Node }
	if state.phase == "build" then
		return waiting("正在编译原生渲染器…", "首次使用需要编译 Rust 渲染器（约 1–3 分钟），之后秒开")
	elseif state.phase == "error" then
		return failed(state)
	elseif state.phase ~= "ready" or state.meta == nil or state.node == nil then
		return waiting("正在解析 " .. state.name .. "…", state.path)
	end
	local meta = state.meta :: model.Meta
	local doc = document(state)
	local body: Node = doc
	if meta.kind == "docx" and state.outline then
		body = ui.el("div", { class = "op-split", key = "split" }, { outline_panel(meta, state.part), doc })
	end
	return { main = ui.col({ body }), dock = dock(state, meta) }
end

function M.title(state: State): string
	return state.name
end

return M
