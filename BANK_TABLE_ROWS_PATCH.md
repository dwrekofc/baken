# Empty table row extension

Based on owned `e1c4112f2e66fb7321dd173569eb196da38bb162`. The existing normal
`pdb::write` path is unchanged. A separate fallible API accepts caller-owned
row callbacks only for existing empty tables, rejecting unknown/duplicate/
already-populated targets. It retains the exact established model/table styles,
sequences, page/presence engine and rollover callback contract. No raw records
are relocated, no table inventory is guessed or replaced, and no decoder/DSP/
ordinary export serialization is changed. Domain-specific bank rows remain the
consumer's responsibility. New unit tests cover byte-exact neutral output,
populated/unknown/duplicate refusal, rollover and fallible/oversized rows.
