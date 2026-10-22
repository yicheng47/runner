ALTER TABLE roles
    ADD COLUMN avatar_color TEXT
    CHECK (
        avatar_color IS NULL OR (
            length(avatar_color) = 7
            AND avatar_color GLOB '#[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
        )
    );
