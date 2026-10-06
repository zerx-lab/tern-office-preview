--!strict
-- Window half: opening a Word / Excel file anywhere in Tern (Files pane, palette, a path clicked
-- in a terminal, Finder, `tern open`) shows it in an Office preview block instead of a text
-- block, which can't show these zip packages anyway.

local KIND = "office-preview.doc"

-- Lower-cased extensions the renderer reads.
local EXTENSIONS = {
	docx = true,
	docm = true,
	dotx = true,
	dotm = true,
	xlsx = true,
	xlsm = true,
	xltx = true,
	xltm = true,
}

local function previewable(path: string): boolean
	local name = string.match(path, "([^/\\]+)$") or path
	-- Office's lock files (~$report.docx) are not documents.
	if string.sub(name, 1, 2) == "~$" then
		return false
	end
	local ext = string.match(name, "%.([%w]+)$")
	return ext ~= nil and EXTENSIONS[string.lower(ext)] == true
end

tern.route.open(function(req, _cx)
	if not previewable(req.path) then
		return nil
	end
	return { block = KIND, args = { req.path } }
end)
