import argparse
import sys


def non_negative(value):
    n = int(value)
    if n < 0:
        raise argparse.ArgumentTypeError("must be non-negative")
    return n


def main(argv, out=sys.stdout):
    parser = argparse.ArgumentParser(prog="lines")
    parser.add_argument("path")
    parser.add_argument("--limit", type=non_negative)
    args = parser.parse_args(argv)
    with open(args.path) as f:
        for i, line in enumerate(f):
            if args.limit is not None and i >= args.limit:
                break
            out.write(line)


if __name__ == "__main__":
    main(sys.argv[1:])
