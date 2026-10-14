# Reusable wire-engine qualification

The independent `wire` workspace imports Genet's active Netfetcher HTTP engine
with its original unfiltered source commit retained in the Murm ancestry. Its
request uses protocol-level `http::Method`; Genet adapts its existing public
contract and retains web policy. The raw WebSocket connector receives an
already assembled handshake. No Genet/Netfetcher back-edge exists.

The source-bound software receipt covers HTTP/1 raw redirect/headers/encoded
bytes, HTTP/2 over TLS with separate pools and trust rejection, HTTP/3 GET/POST,
and raw WebSocket carriage on both Rust compilers. Genet's 83 policy cases,
its real JS Fetch host and Mere's 16 Fetch/crawler cases demonstrate the
receiving boundary before supplier publication. Those first host runs use
task-local supplier paths; the receiving repositories record final immutable
Git checks after publication. No such path is a shipping dependency.

The focused WPT host omits the WebGL fixture, now explicitly selected through
`webgl-tests`. The initial broad command encountered existing macOS compositor
compiler errors; this extraction repairs no renderer and claims no GPU or
presentation result. Default Netfetcher features and public APIs stay intact.
`qualification.json` records 193 passing executions, not distinct cases,
plus compiler/feature-cone receipts and the import rehearsal.
