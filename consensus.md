# Ghost Chain Proto - Hybrid Consensus Logic

## Summary
Hybrid PoW (for distribution/security) + PoS (for finality/low energy).

## Mechanism
- Blocks are mined (PoW) to provide entropy and sybil resistance.
- A committee of Stakers (PoS) must sign off on the block for finality.
- Substrate-based architecture.

## Ghost Features
- Deterministic finality within 1 block.
- "Spectral" transactions that clear off-chain and settle via ZK-proofs.
