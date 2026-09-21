import argparse
import sys


def main(argv, out=sys.stdout):
    parser = argparse.ArgumentParser(prog="lines")
    parser.add_argument("path")
    args = parser.parse_args(argv)
    with open(args.path) as f:
        for line in f:
            out.write(line)


if __name__ == "__main__":
    main(sys.argv[1:])
