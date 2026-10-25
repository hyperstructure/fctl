-- Boxes with flowers carry two VAT rates.
ALTER TABLE box_items ADD COLUMN vat_rate NUMERIC(4,2) NOT NULL DEFAULT 12.00;
