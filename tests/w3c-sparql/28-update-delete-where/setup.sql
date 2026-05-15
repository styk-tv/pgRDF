-- 28-update-delete-where / setup.sql
--
-- Phase C slice 76 — pre-stage rows in the default graph so the
-- subsequent DELETE WHERE has something to match and remove. We
-- intentionally seed four triples in two different shapes:
--
--   ex:alice ex:age "30"
--   ex:alice ex:name "Alice"
--   ex:bob   ex:age "40"
--   ex:bob   ex:name "Bob"
--
-- The query then deletes all `?s ex:age ?o` triples. After execution
-- two rows should remain (the `ex:name` triples); the `_update`
-- summary should report `triples_deleted = 2`.

SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:alice ex:age "30" .
ex:alice ex:name "Alice" .
ex:bob   ex:age "40" .
ex:bob   ex:name "Bob" .
', 0);
