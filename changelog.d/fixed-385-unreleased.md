- A claude killed mid token refresh no longer locks every later claude lane out: before an ACP start shed removes
  the record's `stale_lock` (`.oauth_refresh.lock`) from the login's config folder when it is empty and older
  than its window (60 s), and logs a `lock_cleared` event (path, size, age). A lock with content or a fresh one
  stays. `shed doctor` reports a stale one it finds.
