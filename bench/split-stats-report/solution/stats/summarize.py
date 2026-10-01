from dataclasses import dataclass


@dataclass(frozen=True)
class Summary:
    count: int
    total: float
    mean: float
    minimum: float
    maximum: float


def summarize(rows):
    """A Summary of the values for each name, in first-seen order."""
    values = {}
    for name, value in rows:
        values.setdefault(name, []).append(value)
    return {
        name: Summary(len(v), sum(v), sum(v) / len(v), min(v), max(v))
        for name, v in values.items()
    }
