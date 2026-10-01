from stats.parse import parse_rows
from stats.summarize import summarize
from stats.text import pad_left, pad_right


def render(summaries):
    """The report's lines for these summaries."""
    width = max([4] + [len(name) for name in summaries])

    def line(name, count, total, mean):
        cells = [pad_right(name, width), pad_left(count, 5), pad_left(total, 10), pad_left(mean, 10)]
        return "  ".join(cells).rstrip()

    lines = [line("name", "count", "total", "mean")]
    for name, s in sorted(summaries.items(), key=lambda item: (-item[1].total, item[0])):
        lines.append(line(name, str(s.count), f"{s.total:.2f}", f"{s.mean:.2f}"))
    return lines


def report(lines):
    """The report's lines for these "name,value" lines."""
    return render(summarize(parse_rows(lines)))
