-- How a highlight made on the device is drawn. Kept on the device only: Inoreader's annotations
-- carry no style or colour. Existing highlights keep their gray shading.
ALTER TABLE highlights ADD COLUMN style TEXT NOT NULL DEFAULT 'shaded';
ALTER TABLE highlights ADD COLUMN color TEXT NOT NULL DEFAULT 'gray';
