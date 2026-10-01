"""data/rates.csv -> build/rates.json."""

import csv
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main():
    with open(os.path.join(ROOT, "data", "rates.csv"), newline="") as f:
        rates = {row["currency"]: row["per_eur"] for row in csv.DictReader(f)}
    os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
    with open(os.path.join(ROOT, "build", "rates.json"), "w") as f:
        json.dump(rates, f, indent=2, sort_keys=True)


if __name__ == "__main__":
    main()
