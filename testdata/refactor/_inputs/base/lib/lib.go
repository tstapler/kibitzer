package lib

// Foo returns a greeting.
func Foo() string { return helper() + Util() }

func helper() string { return "hi" }
