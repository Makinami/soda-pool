## Considered features and changes

### High
- allow specifying path to basic client module

### Medium
- support gRPC client modifiers (e.g. send_compressed)

### Low
- support alternative constructors of gRPC client (e.g. with_origin)
- support streaming
- support other async runtimes
- return a single struct wrapping channel and providing access to IP address from get_channel
- support for other server endpoints updates beside DNS records
