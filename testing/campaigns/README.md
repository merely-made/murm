# Campaigns

These executable campaign sources are separate from retained historical
receipts. Historical captures and their original source hashes are preserved
under `Code/testing/murm` at their former repository-relative paths.

The RNS 1.5.4 link-echo campaigns select `RNS_ORACLE_PYTHON` when set and
otherwise use the invoking interpreter. They refuse any other RNS version and
create a fresh directory under the ignored `testing/receipts/rns-154-link-echo`
root, or under the caller's `--output-root`. The repin runners require their
declared RNS and LXMF versions and a new `--output` directory.

The Sennet/Tucket reference runner records software gates only. Its two
standalone admission checks use simulated serial I/O; the full runner also
invokes Cargo. The Sennet desktop wrapper is an attended hardware campaign,
requires explicit executables and output inputs, and is not a smoke test.
Choose new output locations for every run and retain failed evidence.
