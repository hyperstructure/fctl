-- The window each stop asked for, as the customer chose it in the shop.
ALTER TABLE stops
    ADD COLUMN window_start TIMESTAMPTZ,
    ADD COLUMN window_end   TIMESTAMPTZ;

UPDATE stops SET window_start = day + TIME '07:00', window_end = day + TIME '12:00'
 WHERE window_start IS NULL;

ALTER TABLE stops
    ALTER COLUMN window_start SET NOT NULL,
    ALTER COLUMN window_end SET NOT NULL;
