package app

import (
	"fmt"

	"example.com/m/lib"
)

// Run prints things.
func Run() {
	w := lib.Widget{N: 1}
	fmt.Println(lib.Foo(), lib.Util(), w.Double())
}
