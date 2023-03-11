"""TODO(rlevasseur): DO NOT SUBMIT without either providing a detailed docstring or
removing it altogether.
"""

from google3.testing.pybase import googletest
from usr.local.google.home.rlevasseur.p.rickeylev.rules_python.examples.hello_world import (
    hello,
)


class HelloTest(googletest.TestCase):
    def test_give_me_a_name(self):
        pass


if __name__ == "__main__":
    googletest.main()
