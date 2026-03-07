"""Utility functions for data processing."""

class DataProcessor:
    def __init__(self, name):
        self.name = name
        self.results = []

    def process(self, data):
        return [x * 2 for x in data]

def helper(items):
    """Filter and transform items."""
    return [str(item).upper() for item in items if item]

def main():
    proc = DataProcessor("default")
    result = proc.process([1, 2, 3])
    print(result)

if __name__ == "__main__":
    main()
