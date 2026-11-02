package app

import "example.com/m/lib"

// Total uses both.
func Total() int { return lib.Keep() + lib.Moved() }
