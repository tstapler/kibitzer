# Label: strict mode flags test_x (helpers lives in another file); precision-first
# mode accepts it because the callee name is assertion-ish.
import helpers


def test_x():
    helpers.check_x(parse(""))


def test_plain():
    helpers.load(parse(""))
