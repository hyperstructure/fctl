-- Projects' icons (fctl#850): the IconMCP glyph people chose for each, with
-- its color and style, as JSON (`ProjectIcon`). NULL: none chosen yet, and
-- the web app draws the one that fits the project's name and description.
ALTER TABLE projects ADD COLUMN icon TEXT;
