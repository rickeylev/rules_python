# Copyright 2024 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#    http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# it might be possible to use __loader__ to find and load
# src.d/rules_python/__init__.py and replace ourselves?

rules_python = __import__("rules_python.src-d.rules_python")

sys.modules["rules_python"] = rules_python

##import pdb; pdb.set_trace()
##del sys.modules[__name__]
##__path__ = [__path__[0] + "/src.d"]
##__import__(__name__)
##ms = importlib.machinery.ModuleSpec(
##        "rules_python",
##        __loader__,
##        origin = __path__[0] + "/src.d/rules_python/__init__.py",
##        is_package = True
##)
##mod = importlib.util.module_from_spec(ms)
##sys.modules["rules_python"] = mod
#import pdb; pdb.set_trace()
#__path__.append(__path__[0] + '/src.d/rules_python')
