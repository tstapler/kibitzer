package lib

// #include <stdlib.h>
import "C"

// Cgo calls into C.
func Cgo() int { return int(C.abs(-1)) }
