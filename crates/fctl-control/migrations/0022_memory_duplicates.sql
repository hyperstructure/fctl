-- The note a stale note repeats (docs/architecture.md#project-memory): an
-- agent, or a person, marks a note that says nothing another current note
-- doesn't a duplicate of that one (`fctl memory duplicate`), which makes it
-- stale and links it here. NULL for other notes, and once the note it
-- repeats is deleted; making the note current again clears it.
ALTER TABLE memories ADD COLUMN duplicate_of TEXT REFERENCES memories(id) ON DELETE SET NULL;
