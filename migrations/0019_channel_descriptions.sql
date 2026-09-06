ALTER TABLE channels
ADD COLUMN description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 500);
