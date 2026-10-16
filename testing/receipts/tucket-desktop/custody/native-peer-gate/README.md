# Peer-ACK gate trial: missing ACK persists

Source `cefe55149d5` keeps the desktop connected until the stock peer's receipt
confirms its return ACK. Both text bodies and the desktop's outbound ACK still
match, but the peer return ACK remains absent. The new predicate times out and
the UI scenario correctly fails. This does not support teardown as a sufficient
explanation for the missing ACK. Both boards restore and the Djinn host stops.

The next candidate schedules inbound protocol replies after a configurable
turnaround, with a default of 200 ms. This matches the [pinned stock ACK delay](https://github.com/meshcore-dev/MeshCore/blob/d929643/src/helpers/BaseChatMesh.cpp#L3-L49).
The physical queue retains the work's activation and expiry fences; readiness
contributes a wake deadline. No physical send can begin before readiness.
This is a timing hypothesis until its own native trial completes.
