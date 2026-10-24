"""A small calculator: numbers, + - * / % ^, unary minus and parentheses,
with precedence, and errors that say where."""
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


class _Parser:
    def __init__(self, text):
        self.text = text
        self.tokens = tokenize(text)
        self.i = 0

    def peek(self):
        return self.tokens[self.i] if self.i < len(self.tokens) else None

    def end_position(self):
        return len(self.text)

    def unexpected(self, token):
        if token is None:
            raise CalcError('unexpected end of input', self.end_position())
        kind, value, pos = token
        raise CalcError(f"unexpected '{self.text[pos]}'", pos)

    def parse(self):
        value = self.sum()
        token = self.peek()
        if token is not None:
            self.unexpected(token)
        return value

    def sum(self):
        value = self.product()
        while (t := self.peek()) and t[0] == 'op' and t[1] in '+-':
            self.i += 1
            right = self.product()
            value = value + right if t[1] == '+' else value - right
        return value

    def product(self):
        value = self.unary()
        while (t := self.peek()) and t[0] == 'op' and t[1] in '*/%':
            self.i += 1
            right = self.unary()
            if t[1] == '*':
                value = value * right
            elif right == 0:
                raise CalcError('division by zero', t[2])
            elif t[1] == '/':
                value = value / right
            else:
                value = value % right
        return value

    def unary(self):
        t = self.peek()
        if t and t[0] == 'op' and t[1] == '-':
            self.i += 1
            return -self.unary()
        return self.power()

    def power(self):
        base = self.atom()
        t = self.peek()
        if t and t[0] == 'op' and t[1] == '^':
            self.i += 1
            return base ** self.unary()
        return base

    def atom(self):
        t = self.peek()
        if t is None:
            self.unexpected(None)
        kind, value, pos = t
        if kind == 'num':
            self.i += 1
            return value
        if value == '(':
            self.i += 1
            inner = self.sum()
            close = self.peek()
            if close is None:
                raise CalcError("expected ')'", self.end_position())
            if close[1] != ')':
                raise CalcError("expected ')'", close[2])
            self.i += 1
            return inner
        self.unexpected(t)


def evaluate(text):
    return _Parser(text).parse()
