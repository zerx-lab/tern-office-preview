-- A scripted stand-in for the `tern` global, enough to drive the plugin's host and window
-- halves under the standalone `luau` CLI: JSON, an in-memory fs/kv, queued process callbacks
-- answered by test handlers, tree-building ui builders, blob and route capture.

local F = {}

-- ── JSON ────────────────────────────────────────────────────────────────────

local NULL = setmetatable({}, { __tostring = function()
	return "null"
end })
local ARRAYS = setmetatable({}, { __mode = "k" })

local function encode(v: any, out: { string })
	local t = type(v)
	if v == nil or v == NULL then
		table.insert(out, "null")
	elseif t == "boolean" then
		table.insert(out, tostring(v))
	elseif t == "number" then
		if v == math.floor(v) and math.abs(v) < 2 ^ 53 then
			table.insert(out, string.format("%d", v))
		else
			table.insert(out, string.format("%.17g", v))
		end
	elseif t == "string" then
		table.insert(out, '"' .. string.gsub(v, '[%c"\\]', function(c)
			local map = { ['"'] = '\\"', ["\\"] = "\\\\", ["\n"] = "\\n", ["\r"] = "\\r", ["\t"] = "\\t" }
			return map[c] or string.format("\\u%04x", string.byte(c))
		end) .. '"')
	elseif t == "table" then
		local n = #v
		local is_array = ARRAYS[v] or (n > 0 and next(v, n > 0 and n or nil) == nil)
		if is_array or (n > 0) then
			table.insert(out, "[")
			for i = 1, n do
				if i > 1 then
					table.insert(out, ",")
				end
				encode(v[i], out)
			end
			table.insert(out, "]")
		else
			local keys = {}
			for k in v do
				table.insert(keys, tostring(k))
			end
			table.sort(keys)
			table.insert(out, "{")
			for i, k in keys do
				if i > 1 then
					table.insert(out, ",")
				end
				encode(k, out)
				table.insert(out, ":")
				encode(v[k] ~= nil and v[k] or v[tonumber(k)], out)
			end
			table.insert(out, "}")
		end
	else
		error("json encode: " .. t)
	end
end

local function decode(text: string): any
	local pos = 1
	local function ws()
		pos = string.find(text, "[^ \t\r\n]", pos) or #text + 1
	end
	local value
	local function str(): string
		pos += 1
		local out = {}
		while true do
			local c = string.sub(text, pos, pos)
			if c == '"' then
				pos += 1
				return table.concat(out)
			elseif c == "\\" then
				local e = string.sub(text, pos + 1, pos + 1)
				local map = { n = "\n", r = "\r", t = "\t", b = "\b", f = "\f", ["/"] = "/", ["\\"] = "\\", ['"'] = '"' }
				if e == "u" then
					table.insert(out, utf8.char(tonumber(string.sub(text, pos + 2, pos + 5), 16) :: number))
					pos += 6
				else
					table.insert(out, map[e])
					pos += 2
				end
			elseif c == "" then
				error("json decode: unterminated string")
			else
				table.insert(out, c)
				pos += 1
			end
		end
	end
	function value(): any
		ws()
		local c = string.sub(text, pos, pos)
		if c == "{" then
			pos += 1
			local obj = {}
			ws()
			if string.sub(text, pos, pos) == "}" then
				pos += 1
				return obj
			end
			while true do
				ws()
				local k = str()
				ws()
				pos += 1 -- :
				local v = value()
				if v ~= NULL then
					obj[k] = v
				end
				ws()
				local d = string.sub(text, pos, pos)
				pos += 1
				if d == "}" then
					return obj
				end
			end
		elseif c == "[" then
			pos += 1
			local arr = {}
			ARRAYS[arr] = true
			ws()
			if string.sub(text, pos, pos) == "]" then
				pos += 1
				return arr
			end
			while true do
				table.insert(arr, value())
				ws()
				local d = string.sub(text, pos, pos)
				pos += 1
				if d == "]" then
					return arr
				end
			end
		elseif c == '"' then
			return str()
		elseif string.sub(text, pos, pos + 3) == "true" then
			pos += 4
			return true
		elseif string.sub(text, pos, pos + 4) == "false" then
			pos += 5
			return false
		elseif string.sub(text, pos, pos + 3) == "null" then
			pos += 4
			return NULL
		else
			local num = string.match(text, "^-?%d+%.?%d*[eE]?[-+]?%d*", pos)
			if not num then
				error("json decode: unexpected " .. c .. " at " .. pos)
			end
			pos += #num
			return tonumber(num)
		end
	end
	local v = value()
	ws()
	if pos <= #text then
		error("json decode: trailing data")
	end
	return v
end

-- ── runtime ─────────────────────────────────────────────────────────────────

function F.new(opts: { [string]: any }?)
	local o = opts or {}
	local env = {
		files = {} :: { [string]: string },
		kv = {} :: { [string]: any },
		queue = {} :: { () -> () },
		timers = {} :: { { at: number, fn: (any) -> (), cancelled: boolean } },
		clock = 0,
		process = function(_argv: { string }, _opts: any): any
			return { status = 127, stdout = "", stderr = "not scripted", timed_out = false }
		end,
		fetch = function(_url: string, _opts: any): any
			return { status = 0, headers = {}, body = "", error = "not scripted", timed_out = false }
		end,
		calls = { process = {}, fetch = {} } :: { process: { any }, fetch: { any } },
		pending = {} :: { [string]: (any) -> () },
		panes = { 1 } :: { number },
		blocks = {} :: { [string]: any },
		commands = {} :: { [string]: any },
		routes = {} :: { (any, any) -> any },
		blobs = {} :: { { bytes: string, mime: string } },
		status_fn = nil :: any,
		refreshed = 0,
		toasts = {} :: { any },
		envvars = o.env or { HOME = "/home/test", USER = "test" },
		os = o.os or "macos",
	}

	local function node(k: string, p: any?, c: any?): any
		if c ~= nil then
			assert(type(c) == "table" and c.k == nil, "children must be a list of nodes for " .. k)
			local n = 0
			for i in c do
				assert(type(i) == "number", k .. ": non-array children")
				n = math.max(n, i)
			end
			for i = 1, n do
				assert(type(c[i]) == "table" and type(c[i].k) == "string", k .. ": child " .. i .. " is not a node")
			end
		end
		local t: any = { k = k }
		if p and next(p) then
			t.p = p
		end
		if c and #c > 0 then
			t.c = c
		end
		return t
	end
	local function spans(s: any): any
		if type(s) == "string" then
			return { { t = s } }
		elseif type(s) == "table" and s.t then
			return { s }
		end
		for i, v in s or {} do
			assert(type(v) == "string" or (type(v) == "table" and type(v.t) == "string"), "span " .. i .. " is not a span")
		end
		return s or {}
	end

	local ui = {
		span = function(t: string, s: string?)
			assert(type(t) == "string", "span text")
			return { t = t, s = if s and s ~= "" then s else nil }
		end,
		text = function(s: any)
			return node("text", { spans = spans(s) })
		end,
		col = function(c: any)
			return node("col", { gap = "sm" }, c)
		end,
		row = function(c: any)
			return node("row", { gap = "sm" }, c)
		end,
		card = function(head: any, c: any)
			return node("card", { head = spans(head) }, c)
		end,
		lines = function(c: any)
			return node("col", { gap = "none" }, c)
		end,
		badge = function(t: string, tone: string)
			return node("badge", { text = t, tone = tone })
		end,
		md = function(t: string)
			return node("md", { text = t })
		end,
		el = function(tag: string, p: any?, c: any?)
			local props: any = { tag = tag }
			for k, v in p or {} do
				props[k] = v
			end
			return node("el", props, c)
		end,
		node = function(k: string, p: any?, c: any?)
			return node(k, p, c)
		end,
	}

	local tern: any = {
		json = {
			encode = function(v: any, _pretty: boolean?)
				local out = {}
				encode(v, out)
				return table.concat(out)
			end,
			decode = decode,
			array = function(t: any?)
				local a = t or {}
				ARRAYS[a] = true
				return a
			end,
			null = NULL,
		},
		fs = {
			read = function(path: string)
				local v = env.files[path]
				if v == nil then
					error("tern.fs.read " .. path .. ": No such file or directory")
				end
				return v
			end,
			write = function(path: string, text: string)
				env.files[path] = text
			end,
			exists = function(path: string)
				return env.files[path] ~= nil
			end,
			mkdir = function(_path: string) end,
		},
		kv = {
			get = function(k: string)
				return env.kv[k]
			end,
			set = function(k: string, v: any)
				env.kv[k] = v
			end,
		},
		process = {
			run = function(argv: { string }, opts: any, cb: (any) -> ())
				if type(opts) == "function" then
					opts, cb = nil, opts
				end
				table.insert(env.calls.process, { argv = argv, opts = opts })
				local r = env.process(argv, opts or {})
				if r == "pending" then
					-- The test completes it later through env.pending[program].
					env.pending[argv[1]] = cb
					return
				end
				table.insert(env.queue, function()
					cb(r)
				end)
			end,
		},
		fetch = function(url: string, opts: any, cb: (any) -> ())
			table.insert(env.calls.fetch, { url = url, opts = opts })
			local r = env.fetch(url, opts or {})
			table.insert(env.queue, function()
				cb(r)
			end)
		end,
		timer = function(ms: number, fn: (any) -> ())
			local t = { at = env.clock + ms, fn = fn, cancelled = false }
			table.insert(env.timers, t)
			return {
				cancel = function(self: any)
					assert(type(self) == "table", "bad argument 'self' to 'Handle.cancel'")
					t.cancelled = true
				end,
			}
		end,
		now = function()
			return env.clock
		end,
		getenv = function(name: string)
			return env.envvars[name]
		end,
		runtime = { os = env.os, jit = false },
		plugin = { id = "office-preview", name = "Office 预览", dir = "/plugin", data = "/data" },
		log = {
			warn = function(...) end,
			info = function(...) end,
			debug = function(...) end,
			error = function(...) end,
		},
		pane = {
			list = function()
				local out = {}
				for _, p in env.panes do
					table.insert(out, { pane = p, title = "" })
				end
				return out
			end,
		},
		block = {
			define = function(id: string, def: any)
				env.blocks[id] = def
			end,
		},
		command = function(def: any)
			env.commands[def.id] = def
		end,
		route = {
			open = function(fn: any)
				table.insert(env.routes, fn)
			end,
		},
		ui = ui,
	}
	env.tern = tern

	-- Runs queued process/fetch callbacks until none are left.
	function env.drain()
		local guard = 0
		while #env.queue > 0 do
			guard += 1
			assert(guard < 10000, "callback storm")
			local fn = table.remove(env.queue, 1) :: () -> ()
			fn()
		end
	end

	-- Moves the clock and fires due timers (draining callbacks after each).
	function env.advance(ms: number)
		local target = env.clock + ms
		while true do
			env.drain()
			local best, idx = nil, nil
			for i, t in env.timers do
				if not t.cancelled and t.at <= target and (best == nil or t.at < best.at) then
					best, idx = t, i
				end
			end
			if not best then
				break
			end
			table.remove(env.timers, idx)
			env.clock = best.at
			best.fn(nil)
		end
		env.clock = target
		env.drain()
	end

	-- A block cx that counts renders and records toasts and blobs.
	function env.cx(pane: number): any
		local cx: any = { pane = pane, cols = 100, rows = 40, renders = 0 }
		function cx:render()
			cx.renders += 1
		end
		function cx:blob(bytes: string, mime: string): string
			table.insert(env.blobs, { bytes = bytes, mime = mime })
			return "blob-" .. #env.blobs
		end
		function cx:toast(level: string, text: string, sub: string?)
			table.insert(env.toasts, { level = level, text = text, sub = sub })
		end
		function cx:open(url: string)
			cx.opened = url
		end
		function cx:copy(text: string)
			cx.copied = text
		end
		return cx
	end

	return env
end

F.NULL = NULL
F.decode = decode
return F
