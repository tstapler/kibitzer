// Package multi_import is the fan_in_out regression fixture: both files import the same
// package, so Ce(multi_import) must be 1, not 2.
package multi_import

import "dogfood.example/app/domain"

func A() domain.Order { return domain.Order{} }
