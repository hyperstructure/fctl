-- When each farm hands over its harvest at the depot.
CREATE TABLE farm_pickup_windows (
    farm_id      BIGINT NOT NULL REFERENCES farms (id),
    weekday      SMALLINT NOT NULL CHECK (weekday BETWEEN 1 AND 7),
    starts_at    TIME NOT NULL,
    ends_at      TIME NOT NULL,
    PRIMARY KEY (farm_id, weekday)
);
