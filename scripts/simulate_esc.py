import math
import collections

def calculate_shannon_entropy(producers):
    if not producers: return 0
    counts = collections.Counter(producers)
    total = len(producers)
    entropy = 0
    for count in counts.values():
        p_i = count / total
        entropy -= p_i * math.log2(p_i)
    return entropy

def simulate_esc():
    # Healthy threshold from our pallet: 4.0
    threshold = 4.0
    current_difficulty = 1000000
    
    print(f"--- ESC Simulation (Threshold: {threshold}) ---")
    
    # Scene 1: Decentralized (100 different producers)
    producers_decentralized = [f"user_{i}" for i in range(100)]
    ent_dec = calculate_shannon_entropy(producers_decentralized)
    # adjustment = (5*100)/5 = 100 (no time change)
    # penalty = (4.0 - 6.64) -> negative -> 0
    print(f"Decentralized Entropy: {ent_dec:.4f} | Difficulty: {current_difficulty}")

    # Scene 2: Centralized (1 miner taking 80% of blocks)
    producers_centralized = ["whale"] * 80 + [f"user_{i}" for i in range(20)]
    ent_cen = calculate_shannon_entropy(producers_centralized)
    
    # Manual check of our pallet logic:
    # entropy_penalty = ((4,000,000 - 1,180,000) * 100) / 4,000,000 = ~70
    # adjustment_factor = 100 + 70 = 170
    # new_diff = (current_difficulty * 170) / 100
    
    penalty_pct = max(0, (threshold - ent_cen) / threshold) * 100
    new_difficulty = current_difficulty * (100 + penalty_pct) / 100
    
    print(f"Centralized Entropy: {ent_cen:.4f} | Penalty: +{penalty_pct:.1f}%")
    print(f"Steered Difficulty: {int(new_difficulty)} (Harder to mine for the whale)")

if __name__ == "__main__":
    simulate_esc()
