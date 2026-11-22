- `shed events` no longer drops agent text chunks for a reader that falls more than 256 chunks behind
  but stays within the 4,096-event window: every chunk in the last 4,096 events is kept until it is read,
  and only a reader past that window is dropped, with a `lagged` line.
