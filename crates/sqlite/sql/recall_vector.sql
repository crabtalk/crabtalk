SELECT key, value, metadata, created_at, accessed_at, access_count, embedding
FROM memories
WHERE embedding IS NOT NULL
