# Economy Module Analysis

Looking at the economy.rs module, I can see this is a comprehensive resource management system ported from Python with the following key components:

## Resource Registry (RESOURCES)
- 60+ resources categorized by type: Crops, Livestock, Forestry, Mining, Fishing, Subterranean, Food Products, Manufactured, Luxury
- Each resource has properties:
  - Category (determines tier)
  - Tier (1-5, with different storage requirements)
  - Spoil rate (fraction lost per day in storage)
  - Bulk (storage space per unit relative to grain)
  - Pool (which storage compartment it occupies)
  - Edible flag (safe to eat directly)
  - Luxury flag (status good, not sustenance)

## Storage System
- Four storage pools:
  1. Household (Crops/Food Products/Fishing/Firewood)
  2. Durable (Mining/Forestry except Firewood/Manufactured)
  3. Other (Luxury Goods/Gold)
  4. Feed (Fodder/Manure/Guano)

## Production Conversion
- RECIPES table defines which resources can be converted to others
- Conversion rate caps:
  - CONVERSION_RATE_CAP: 30 units/day for staple goods
  - LUXURY_CONVERSION_RATE_CAP: 2 units/day for luxury goods (locked until year 1)
- Alternatives within recipes work on first-available basis
- Storage throttling: production tapers as pools fill (85% start, 15% floor)

## Spoilage System
- Resources decay at their spoil_rate per day
- Overflow spoilage: pools over capacity have additional decay rate
- Gold never spoils and occupies Vault space

## Key Constants
- STARTING_GOLD_PER_FACTION: 4000.0 (split between two towns)
- Storage capacities are base + building tier bonuses
- STORAGE_THROTTLE_START: 0.85 (taper starts at 85% full)
- STORAGE_THROTTLE_FLOOR: 0.15 (minimum production rate)

## Settlement Context
From build.rs, I can see settlements need:
- Storage tiers for granary/warehouse/vault
- Treasury system for funding construction
- Tax income collection