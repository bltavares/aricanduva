-- Create metadata table for IPFS content
CREATE TABLE IF NOT EXISTS metadata (
    cid TEXT NOT NULL,
    bucket TEXT NOT NULL DEFAULT '',
    object_key TEXT NOT NULL DEFAULT '',
    content_type TEXT NOT NULL DEFAULT 'application/octect-stream',
    size INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    PRIMARY KEY (bucket, object_key)
);

-- Create index for faster CID lookups
CREATE INDEX IF NOT EXISTS idx_metadata_cid ON metadata(cid);
CREATE INDEX IF NOT EXISTS idx_metadata_bucket ON metadata(bucket);
CREATE UNIQUE INDEX IF NOT EXISTS idx_metadata_bucket_key ON metadata(bucket, object_key);
