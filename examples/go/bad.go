package bad

import (
	"fmt"
	_ "net/http/pprof"
)

// go-ignored-error: discards the error return instead of checking it.
func readConfig(path string) string {
	result, _ := loadFile(path)
	return result
}

func loadFile(path string) (string, error) {
	return path, nil
}

// go-error-context: bare passthrough, even though this file already wraps
// errors elsewhere with fmt.Errorf("%w", ...) below.
func openConn(addr string) error {
	if err := dial(addr); err != nil {
		return err
	}
	return nil
}

func dial(addr string) error {
	return nil
}

func wrapExample(addr string) error {
	if err := dial(addr); err != nil {
		return fmt.Errorf("dialing %s: %w", addr, err)
	}
	return nil
}

// primitive-obsession + long-parameter-list: six same-typed primitive
// parameters instead of a value object.
func createUser(firstName string, lastName string, email string, phone string, street string, city string) string {
	return firstName + lastName + email + phone + street + city
}

// deep-nesting: five levels of nested control flow (limit is 4).
func classify(n int) string {
	if n > 0 {
		if n > 10 {
			if n > 100 {
				if n > 1000 {
					if n > 10000 {
						return "huge"
					}
					return "big"
				}
				return "medium"
			}
			return "small"
		}
		return "tiny"
	}
	return "non-positive"
}

// long-function: exceeds the 40-line limit.
func processOrder(id int) int {
	total := 0
	total += 1
	total += 2
	total += 3
	total += 4
	total += 5
	total += 6
	total += 7
	total += 8
	total += 9
	total += 10
	total += 11
	total += 12
	total += 13
	total += 14
	total += 15
	total += 16
	total += 17
	total += 18
	total += 19
	total += 20
	total += 21
	total += 22
	total += 23
	total += 24
	total += 25
	total += 26
	total += 27
	total += 28
	total += 29
	total += 30
	total += 31
	total += 32
	total += 33
	total += 34
	total += 35
	total += 36
	total += 37
	total += 38
	total += 39
	total += 40
	total += 41
	return total + id
}

// duplicate-code: this block is repeated three times below, verbatim.
func summarizeA(items []int) int {
	sum := 0
	for _, item := range items {
		if item > 0 {
			sum += item * 2
		}
	}
	return sum
}

func summarizeB(items []int) int {
	sum := 0
	for _, item := range items {
		if item > 0 {
			sum += item * 2
		}
	}
	return sum
}

func summarizeC(items []int) int {
	sum := 0
	for _, item := range items {
		if item > 0 {
			sum += item * 2
		}
	}
	return sum
}
