class Migration:
    """One step: a name, what it does, and how to undo it."""

    def __init__(self, name, up, down):
        if not name:
            raise ValueError('a migration needs a name')
        self.name = name
        self.up = up
        self.down = down

    def __repr__(self):
        return f'Migration({self.name!r})'
