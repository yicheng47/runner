-- Migration 0024: rename the seeded crew from "Peer coding crew" to
-- "Pair coding crew" (#676). The seed runs once per database, so the
-- literal change in `seed::seed_default_crew` only reaches fresh
-- installs; this UPDATE carries existing ones.
--
-- Keyed on the seed's pinned crew ID *and* the old name, so a crew the
-- user has since renamed keeps their name. The ID itself still spells
-- PEERCODING: it is a primary key already present in user databases and
-- is never shown. Stored role prompts are left alone for the same
-- reason the 0002 rewrite scoped itself to fixed IDs — users may have
-- edited them.

UPDATE crews
SET name = 'Pair coding crew'
WHERE id = '01K000DEFAULT000PEERCODING01'
  AND name = 'Peer coding crew';
