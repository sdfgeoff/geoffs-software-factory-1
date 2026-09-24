ALTER TABLE agents ADD COLUMN instructions TEXT NOT NULL DEFAULT ''
    CHECK (length(instructions) <= 10000);
