class Migration:
    """One step: a name, what it does, how to undo it (or not), and what it
    needs applied first."""

    def __init__(self, name, up, down=None, depends_on=()):
        if not name:
            raise ValueError('a migration needs a name')
        self.name = name
        self.up = up
        self.down = down
        self.depends_on = list(depends_on)

    def __repr__(self):
        return f'Migration({self.name!r})'
