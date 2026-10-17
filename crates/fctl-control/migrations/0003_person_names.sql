-- First names (all of them: "Carl Oscar") and the last name, next to the
-- full name everyone sees. Greetings use given_name.
ALTER TABLE users ADD COLUMN given_name TEXT;
ALTER TABLE users ADD COLUMN family_name TEXT;

-- Existing accounts: the last word becomes the last name, the rest the first
-- names (rtrim strips the trailing run of non-space characters). People can
-- correct it in Settings.
UPDATE users SET
  given_name = CASE WHEN instr(name, ' ') > 0 THEN rtrim(rtrim(name, replace(name, ' ', ''))) ELSE name END,
  family_name = CASE WHEN instr(name, ' ') > 0 THEN substr(name, length(rtrim(name, replace(name, ' ', ''))) + 1) ELSE NULL END
WHERE given_name IS NULL AND email NOT LIKE 'deleted-%@deleted.invalid';
