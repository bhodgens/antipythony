def classify(n):
    match n:
        case int(x) if x < 0:
            return "negative"
        case int(x) if x == 0:
            return "zero"
        case int(x):
            return "positive"
