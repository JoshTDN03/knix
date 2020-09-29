#   Copyright 2020 The KNIX Authors
#
#   Licensed under the Apache License, Version 2.0 (the "License");
#   you may not use this file except in compliance with the License.
#   You may obtain a copy of the License at
#
#       http://www.apache.org/licenses/LICENSE-2.0
#
#   Unless required by applicable law or agreed to in writing, software
#   distributed under the License is distributed on an "AS IS" BASIS,
#   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#   See the License for the specific language governing permissions and
#   limitations under the License.

import unittest
import os, sys
import json

sys.path.append("../")
from mfn_test_utils import MFNTest

class DlibTest(unittest.TestCase):

    """ Example ASL state test with Dlib

    """
    def test_dlib(self):
        """  testing dlib """

        inp1 = '"abc"'
        #res1 = '"Hello from Tensorflow 2.1.0"'

        res1 = '"GPU available: True"'

        testtuplelist =[(inp1, res1)]

        test = MFNTest(test_name = "Dlib_Test", num_gpu = 1)
        test.exec_tests(testtuplelist)

