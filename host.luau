--!strict
-- Block `office-preview.doc`: previews the .docx / .xlsx given as its argument.
--
-- The Rust renderer (native/, run through tern.process.run) turns the file into part files of
-- view nodes in <data>/cache; the block shows one part at a time and sends the part's pictures
-- as blobs. Keys: ] [ (or n p) parts · } { (or tab) sheets · g G first/last · + - 0 zoom ·
-- o outline · r reload · b rebuild the renderer. Sheets: click / arrows select cells like
-- Office, Esc clears, h toggles the row/column crosshair (remembered in tern.kv), y copies.

local grid = require("./lib/grid")
local model = require("./lib/model")
local native = require("./lib/native")
local view = require("./lib/view")

-- tern.kv key of the crosshair switch, shared by every block and kept across restarts.
local CROSS_KEY = "crosshair"

type State = view.State

-- Whether pane `pane` still runs this block.
local function running(pane: number): boolean
	for _, p in tern.pane.list() do
		if p.pane == pane then
			return true
		end
	end
	return false
end

-- Loads the current part's nodes and sends its pictures; on failure shows the error.
local function load_part(state: State, cx: BlockCx)
	local meta = state.meta
	local dir = state.dir
	if meta == nil or dir == nil then
		return
	end
	local parts = model.parts(meta, state.sheet)
	state.part = model.clamp(state.part, 1, #parts)
	local part = parts[state.part]
	if part == nil then
		state.phase, state.error = "error", "文档没有可显示的内容"
		return
	end
	local ok, node, images = pcall(native.read_part, dir, part.file)
	if not ok then
		state.phase, state.error = "error", "读取渲染结果失败：" .. tostring(node)
		return
	end
	for _, img in images :: { { file: string, mime: string } } do
		local full = dir .. "/" .. img.file
		if state.blobs[full] == nil then
			local read_ok, bytes = pcall(tern.fs.read, full)
			if read_ok then
				state.blobs[full] = cx:blob(bytes, img.mime)
			end
		end
	end
	local by_file: { [string]: string } = {}
	for _, img in images :: { { file: string, mime: string } } do
		by_file[img.file] = state.blobs[dir .. "/" .. img.file]
	end
	model.attach(node, by_file)
	local p = node.p or {}
	node.p = p
	state.node_class = p.class or "opv"
	state.node_key = "s" .. state.sheet .. "-" .. part.file
	state.node = node
	state.grid = if meta.kind == "xlsx" then grid.index(node) else nil
	if state.grid then
		grid.mark(state.grid :: grid.Index, state.sel, state.cross)
	end
	state.phase, state.error = "ready", nil
end

-- Whether `sheet` is one of the workbook's tabs.
local function is_tab(meta: model.Meta, sheet: number): boolean
	for _, i in model.tabs(meta) do
		if i == sheet then
			return true
		end
	end
	return false
end

-- Renders the file and shows the current part. While a reload runs, the old part stays up.
local function render(state: State, cx: BlockCx)
	state.gen += 1
	local gen = state.gen
	if state.node == nil then
		state.phase = "render"
	end
	native.render(state.path, function(res, err)
		if gen ~= state.gen or not running(cx.pane) then
			return
		end
		if res == nil then
			state.phase, state.error = "error", err
		else
			state.dir, state.meta = res.dir, res.meta :: any
			local meta = state.meta :: model.Meta
			if meta.kind ~= "xlsx" then
				state.sheet = 1
			elseif not is_tab(meta, state.sheet) then
				state.sheet, state.part = model.first_sheet(meta), 1
			end
			load_part(state, cx)
		end
		cx:render()
	end)
end

-- Renders, building (or with `rebuild`, rebuilding) the native renderer first when needed.
local function start(state: State, cx: BlockCx, rebuild: boolean)
	local ready = native.status() == "ready"
	if ready and not rebuild then
		render(state, cx)
		return
	end
	if not native.can_build() then
		if ready then
			render(state, cx)
		else
			state.phase, state.error = "error",
				"此平台没有可用的预编译渲染器（" .. tern.runtime.os .. "），也无法从源码构建：\n请安装带预编译渲染器的发布包（scripts/package.sh 生成）"
		end
		return
	end
	state.gen += 1
	local gen = state.gen
	state.phase, state.node = "build", nil
	native.build(function(ok, err)
		if gen ~= state.gen or not running(cx.pane) then
			return
		end
		if ok then
			render(state, cx)
		else
			state.phase, state.error = "error", "构建渲染器失败：\n" .. (err or "")
		end
		cx:render()
	end)
end

local function go_part(state: State, cx: BlockCx, n: number): boolean
	local meta = state.meta
	if meta == nil or state.phase ~= "ready" then
		return false
	end
	local count = #model.parts(meta, state.sheet)
	local target = model.clamp(n, 1, count)
	if target == state.part then
		return false
	end
	state.part = target
	load_part(state, cx)
	return true
end

local function go_sheet(state: State, cx: BlockCx, sheet: number): boolean
	local meta = state.meta
	if meta == nil or state.phase ~= "ready" or meta.kind ~= "xlsx" or sheet == state.sheet then
		return false
	end
	state.sheet, state.part, state.sel = sheet, 1, nil
	load_part(state, cx)
	return true
end

-- Selects `sel` (nil clears) and re-marks the sheet.
local function choose(state: State, sel: grid.Sel?): boolean
	state.sel = sel
	local ix = state.grid
	if ix then
		grid.mark(ix, sel, state.cross)
	end
	return true
end

-- Moves the selection one cell; past the part's first / last row it continues in the
-- previous / next part, in the same column.
local function move(state: State, cx: BlockCx, dr: number, dc: number): boolean
	local ix = state.grid
	if ix == nil then
		return false
	end
	local nxt, edge = grid.move(ix, state.sel, dr, dc)
	if edge == 0 then
		return choose(state, nxt)
	end
	local cur = state.sel :: grid.Sel
	local active = grid.cell(ix, cur.r, cur.c)
	local last = if active then active.r2 else cur.r
	if not go_part(state, cx, state.part + edge) then
		return false
	end
	local nix = state.grid
	if nix == nil or #nix.rows == 0 then
		return true
	end
	-- Frozen rows repeat at the top of every part; continue below them going down.
	local row = nix.rows[#nix.rows]
	if edge > 0 then
		for _, r in nix.rows do
			if r > last then
				row = r
				break
			end
		end
	end
	local target = grid.cell(nix, row, cur.c) or grid.cell(nix, row, nix.cols[1])
	return choose(state, if target then { kind = "cell", r = target.r, c = target.c } else nil)
end

-- Copies the selected cell's text.
local function copy(state: State, cx: BlockCx): boolean
	local ix, sel = state.grid, state.sel
	if ix == nil or sel == nil or sel.kind ~= "cell" then
		return false
	end
	local cell = grid.cell(ix, sel.r, sel.c)
	if cell == nil then
		return false
	end
	cx:copy(grid.text(cell))
	cx:toast("success", "已复制 " .. grid.address(cell.r, cell.c))
	return true
end

local function toggle_cross(state: State): boolean
	state.cross = not state.cross
	pcall(tern.kv.set, CROSS_KEY, state.cross)
	return choose(state, state.sel)
end

local function zoom(state: State, dir: number): boolean
	local z = if dir == 0 then 100 else model.zoom_step(state.zoom, dir)
	if z == state.zoom then
		return false
	end
	state.zoom = z
	return true
end

local doc: BlockDef<State> = {
	init = function(cx, args, saved)
		local path = (saved and saved.path) or args[1]
		local state: State = {
			path = path or "",
			name = if path then model.basename(path) else "Office 预览",
			phase = "render",
			error = nil,
			dir = nil,
			meta = nil,
			sheet = (saved and saved.sheet) or 0, -- 0: the workbook's active sheet
			part = (saved and saved.part) or 1,
			zoom = (saved and saved.zoom) or 100,
			outline = (saved and saved.outline) or false,
			node = nil,
			node_class = "opv",
			node_key = "p",
			blobs = {},
			gen = 0,
			grid = nil,
			sel = nil,
			cross = select(2, pcall(tern.kv.get, CROSS_KEY)) == true,
		}
		if path == nil or path == "" then
			state.phase, state.error = "error", "没有指定文件：从文件面板选择「Open with Office 预览」，或在终端运行 tern open 文件.docx"
			return state
		end
		start(state, cx, false)
		return state
	end,

	title = function(state)
		return view.title(state)
	end,

	view = function(state, _cx)
		return view.render(state)
	end,

	key = function(state, key, cx)
		local name = key.name
		if key.meta and name == "c" then
			return copy(state, cx)
		end
		if key.meta or key.ctrl then
			return false
		end
		local text = key.text
		-- Printable keys by what they type (shift+] is "}"), the rest by name.
		local ch = if type(text) == "string" and #text == 1 and string.byte(text) > 32 then text else name
		local meta = state.meta
		if state.grid and (name == "up" or name == "down" or name == "left" or name == "right") then
			local dr = if name == "down" then 1 elseif name == "up" then -1 else 0
			local dc = if name == "right" then 1 elseif name == "left" then -1 else 0
			return move(state, cx, dr, dc)
		elseif name == "escape" then
			return if state.sel then choose(state, nil) else false
		elseif ch == "h" and state.grid then
			return toggle_cross(state)
		elseif ch == "y" then
			return copy(state, cx)
		end
		if ch == "r" then
			start(state, cx, false)
		elseif ch == "b" then
			start(state, cx, true)
		elseif ch == "]" or ch == "n" or ch == "right" then
			return go_part(state, cx, state.part + 1)
		elseif ch == "[" or ch == "p" or ch == "left" then
			return go_part(state, cx, state.part - 1)
		elseif ch == "g" or ch == "home" then
			return go_part(state, cx, 1)
		elseif ch == "G" or ch == "end" then
			return go_part(state, cx, math.huge)
		elseif meta and (ch == "}" or (name == "tab" and not key.shift)) then
			return go_sheet(state, cx, model.step_sheet(meta, state.sheet, 1))
		elseif meta and (ch == "{" or (name == "tab" and key.shift)) then
			return go_sheet(state, cx, model.step_sheet(meta, state.sheet, -1))
		elseif ch == "+" or ch == "=" then
			return zoom(state, 1)
		elseif ch == "-" or ch == "_" then
			return zoom(state, -1)
		elseif ch == "0" then
			return zoom(state, 0)
		elseif ch == "o" and meta and meta.kind == "docx" then
			state.outline = not state.outline
		else
			return false
		end
		return true
	end,

	event = function(state, ev, cx)
		if ev.ev == "select" and type(ev.item) == "string" then
			local i = tonumber(ev.item)
			if i then
				go_sheet(state, cx, i)
			end
			return
		end
		if ev.ev ~= "action" then
			return
		end
		local act, value = ev.act, ev.value
		if act == "c" then
			local hit = grid.hit(ev.id)
			if hit then
				choose(state, hit)
			end
			return
		elseif act == "cross" then
			toggle_cross(state)
			return
		end
		if act == "prev" then
			go_part(state, cx, state.part - 1)
		elseif act == "next" then
			go_part(state, cx, state.part + 1)
		elseif act == "goto" then
			go_part(state, cx, tonumber(value) or state.part)
		elseif act == "zoom" then
			zoom(state, tonumber(value) or 0)
		elseif act == "outline" then
			state.outline = not state.outline
		elseif act == "reload" then
			start(state, cx, false)
		end
	end,

	save = function(state)
		return {
			path = state.path,
			sheet = state.sheet,
			part = state.part,
			zoom = state.zoom,
			outline = state.outline,
		}
	end,
}

tern.block.define("doc", doc)
