package lib

var registry = map[string]func(){}

func Register(name string, f func()) { registry[name] = f }
