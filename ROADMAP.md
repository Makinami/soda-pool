## Planed features

### High
- get endpoint information (e.g. url) from ChannelPool
- allow specifying path to basic client module

### Medium
- support gRPC client modifiers (e.g. send_compressed)
- support single channel version for testing

### Low
- support alternative constructors of gRPC client (e.g. with_origin)
- support streaming
- support other async runtimes
- return a single struct wrapping channel and providing access to IP address from get_channel
- support for other server endpoints updates beside DNS records
- support multiple versions of tonic through features
