def parse_rows(lines):
    """`(name, value)` for each "name,value" line."""
    rows = []
    seen_data = False
    for number, line in enumerate(lines, start=1):
        text = line.strip()
        if not text or text.startswith("#"):
            continue
        first = not seen_data
        seen_data = True
        if first and text.replace(" ", "").lower() == "name,value":
            continue
        parts = [part.strip() for part in text.split(",")]
        if len(parts) != 2 or not parts[0]:
            raise ValueError(f"line {number}: expected name,value, got {text!r}")
        try:
            value = float(parts[1])
        except ValueError:
            raise ValueError(f"line {number}: {parts[1]!r} is not a number") from None
        rows.append((parts[0], value))
    return rows
