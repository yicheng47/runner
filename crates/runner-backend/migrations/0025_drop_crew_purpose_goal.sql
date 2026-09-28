-- These crew fields no longer have readers. Mission goals remain on missions;
-- team conventions remain in system_prompt_addendum.
ALTER TABLE crews DROP COLUMN purpose;
ALTER TABLE crews DROP COLUMN goal;
