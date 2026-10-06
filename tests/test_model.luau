-- Pure navigation helpers: zoom steps, sheet tabs and wrapping, blob attachment.

local fake = require("./fake_tern")

local function eq(a: any, b: any, what: string)
	if a ~= b then
		error(string.format("%s: expected %s, got %s", what, tostring(b), tostring(a)), 2)
	end
end

__fresh(fake.new())
local model = require("../lib/model")

eq(model.zoom_step(100, 1), 110, "zoom in")
eq(model.zoom_step(100, -1), 90, "zoom out")
eq(model.zoom_step(200, 1), 200, "zoom in stops at the top")
eq(model.zoom_step(50, -1), 50, "zoom out stops at the bottom")
eq(model.zoom_step(105, -1), 100, "off-grid zoom snaps down")

local meta: any = {
	kind = "xlsx",
	active = 1,
	sheets = {
		{ name = "a", state = "visible", parts = {} },
		{ name = "b", state = "veryHidden", parts = {} },
		{ name = "c", state = "visible", parts = { { file = "s2-0", label = "1–1" } } },
	},
}
eq(#model.tabs(meta), 2, "very hidden sheets are not tabs")
eq(model.first_sheet(meta), 1, "a very hidden active sheet falls back to the first tab")
meta.active = 2
eq(model.first_sheet(meta), 3, "active tab")
eq(model.step_sheet(meta, 3, 1), 1, "wraps forward")
eq(model.step_sheet(meta, 1, -1), 3, "wraps backward")
eq(#model.parts(meta, 3), 1, "sheet parts")
eq(#model.parts({ kind = "docx", parts = { {}, {} } } :: any, 1), 2, "document parts")

local node: any = {
	k = "el",
	c = { { k = "image", p = { src = "img/1.png", w = 3 } }, { k = "el", c = { { k = "image", p = { src = "img/2.svg" } } } } },
}
model.attach(node, { ["img/1.png"] = "b1", ["img/2.svg"] = "b2" })
eq(node.c[1].p.blob, "b1", "first image")
eq(node.c[1].p.src, nil, "src dropped")
eq(node.c[1].p.w, 3, "other props kept")
eq(node.c[2].c[1].p.blob, "b2", "nested image")

eq(model.basename("/a/b/报告.docx"), "报告.docx", "basename")
eq(model.clamp(math.huge, 1, 4), 4, "clamp to last")
eq(model.clamp(3, 1, 0), 1, "empty range")

print("model ok")
