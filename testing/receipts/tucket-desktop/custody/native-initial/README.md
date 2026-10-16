# Initial Djinn native trial: incomplete return acknowledgement

Source `8bd89bedc5d` delivered the desktop's tagged text to stock MeshCore and
observed its matching ACK. The stock return text reached Signalman. The native
UI completed six captures, disconnect, reconnect and exact history assertions.
The stock return remained `HandedToRadio`, with its acknowledgement window
expired. The outer 180-second deadline correctly failed the full trial.

Both boards were restored: COM7's same-boot profile/sync was acknowledged and
COM6's pinned Tulle image, identity/status, region and channel matched. The
isolated Djinn host stopped. Private runtime/wallet files are excluded here.

The scenario disconnected after local receive without waiting for the peer's
return-ACK result. Teardown racing the host-generated ACK is a hypothesis, not
an established cause. The next fixture waits on the matching peer receipt
before disconnect, preserving the full two-sided acceptance condition.

All six captures were reviewed. History and connection states are visible.
The first roundtrip capture still shows a transient handoff; later captures
show the observed protocol ACK. A stale connecting notice is cosmetic work
outside this functional gate. This trial is not a native qualification pass.

`ack-reference.json` independently reproduces stock's expected `9a6bd323` ACK
from public message data using the [pinned stock construction](https://github.com/meshcore-dev/MeshCore/blob/d929643/src/helpers/BaseChatMesh.cpp#L382-L400).
It does not establish whether Tucket transmitted that ACK on air.
