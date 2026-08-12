-- pgrdf 0.6.28 -> 0.6.29: no SQL-surface change. This release fixes the
-- build identity of the ATTESTED ARTIFACT (pgrdf.build_id() baked from the
-- release tag, gate asserts all three identity planes — #112). The catalog
-- objects are identical; this script exists so ALTER EXTENSION pgrdf UPDATE
-- walks the chain without a gap.
SELECT 1;
