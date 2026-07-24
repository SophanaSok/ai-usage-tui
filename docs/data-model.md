# Data Model

The normalized usage event is the contract shared by collectors, aggregation, exports, and the UI.

```text
timestamp
provider
model
category: local | cloud | free | paid | unknown
cost_status: provider_reported | calculated | estimated | free | local | unavailable
request_count
input_tokens
output_tokens
reasoning_tokens
cache_read_tokens
cache_write_tokens
cost
latency
error_status
project
session
source
```

Historical records should retain the pricing snapshot or source used to calculate their cost. Provider adapters should tolerate missing optional fields and preserve the event with an explicit unknown status.

The local journal currently stores usage metadata in `usage_event`. It intentionally excludes prompt and completion content.
