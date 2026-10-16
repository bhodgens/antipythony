def task_group_demo():
    try:
        raise ExceptionGroup("eg", [ValueError("x")])
    except* ValueError as e:
        pass
    except* TypeError:
        pass
