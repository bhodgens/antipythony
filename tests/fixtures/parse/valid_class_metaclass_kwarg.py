class Meta(type):
    pass

class Configured(dict, metaclass=Meta):
    option = "on"
