package lib

import (
	"fmt"
	"strings"
)

const seed = 7

// Hash mixes s with the package seed.
func Hash(s string) string {
	return fmt.Sprint(mix(strings.ToLower(s)))
}

func mix(s string) int { return len(s) * seed }

// Other stays in lib.
func Other() int { return 1 }
