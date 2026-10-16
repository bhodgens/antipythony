def first_of(seq):
    match seq:
        case []:
            return None
        case [head]:
            return head
        case [head, *rest]:
            return (head, rest)
