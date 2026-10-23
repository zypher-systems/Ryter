"""A small calculator: numbers, + - * / and parentheses, evaluated as they come."""
import re

TOKEN = re.compile(r'\s*(?:(\d+\.\d*|\.\d+|\d+)|(\S))')


class CalcError(Exception):
    def __init__(self, message, position):
        super().__init__(message)
        self.message = message
        self.position = position


def tokenize(text):
    """(kind, value, position) for each token: kind is 'num' or 'op'."""
    tokens = []
    pos = 0
    while pos < len(text):
        m = TOKEN.match(text, pos)
        if not m:
            break
        if m.group(1) is not None:
            raw = m.group(1)
            value = float(raw) if '.' in raw else int(raw)
            tokens.append(('num', value, m.start(1)))
        else:
            tokens.append(('op', m.group(2), m.start(2)))
        pos = m.end()
    return tokens


def evaluate(text):
    """Left to right, no precedence: 1+2*3 is 9 here."""
    tokens = tokenize(text)
    if not tokens:
        raise CalcError('unexpected end of input', len(text))
    stack = []
    value = None
    op = None
    for kind, item, pos in tokens:
        if kind == 'num':
            value = item if value is None or op is None else _apply(op, value, item, pos)
            op = None
        elif item == '(':
            stack.append((value, op))
            value, op = None, None
        elif item == ')':
            inner = value
            value, op = stack.pop()
            value = inner if value is None or op is None else _apply(op, value, inner, pos)
            op = None
        elif item in '+-*/':
            op = item
        else:
            raise CalcError(f"unexpected '{item}'", pos)
    return value


def _apply(op, left, right, pos):
    if op == '+':
        return left + right
    if op == '-':
        return left - right
    if op == '*':
        return left * right
    if right == 0:
        raise CalcError('division by zero', pos)
    return left / right
