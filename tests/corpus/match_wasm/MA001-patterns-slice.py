# MA001: capture/wildcard/literal/or/guard/sequence/mapping patterns
m = [1, 2, 3]
match m:
    case [a, b, c]:
        print('seq3', a, b, c)
    case _:
        print('no')

match [1, 2, 3, 4]:
    case [first, *rest]:
        print('star', first, rest)

match {'k': 5, 'j': 6}:
    case {'k': v}:
        print('map', v)

match 7:
    case 1 | 2:
        print('or12')
    case x if x > 5:
        print('guard', x)
    case _:
        print('wild')

data = [('a', 1), ('b', 2)]
match data:
    case [('a', av), ('b', bv)]:
        print('tuples', av, bv)

match 'hello':
    case 'hello':
        print('lit str')

match None:
    case None:
        print('none match')

match 3:
    case 1 | 2 | 3:
        print('or-chain')

point = (5, 7)
match point:
    case (0, 0):
        print('origin')
    case (x, 0):
        print('x-axis', x)
    case (0, y):
        print('y-axis', y)
    case (x, y):
        print('free', x, y)
