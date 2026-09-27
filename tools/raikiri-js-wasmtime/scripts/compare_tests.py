import unittest
from compare import compare
class ComparisonTests(unittest.TestCase):
    def test_duplicate_names_and_status_changes(self):
        native = [{'test_id':'a','error':None,'tests':[{'name':'same','passed':True,'message':''},{'name':'same','passed':False,'message':'FAIL: x'}]}]
        guest = [{'test_id':'a','error':None,'tests':[{'name':'same','passed':True,'message':''},{'name':'same','passed':False,'message':'TIMEOUT: x'}]}]
        self.assertEqual(len(compare(native, guest)['differences']), 1)
    def test_missing_file_and_extra_assertion(self):
        native = [{'test_id':'a','error':None,'tests':[]}]
        self.assertTrue(compare(native, [])['differences'])
    def test_message_only_differences(self):
        a=[{'test_id':'a','error':None,'tests':[{'name':'x','passed':False,'message':'FAIL: a'}]}]
        b=[{'test_id':'a','error':None,'tests':[{'name':'x','passed':False,'message':'FAIL: b'}]}]
        report=compare(a,b)
        self.assertFalse(report['differences'])
        self.assertEqual(len(report['message_differences']),1)
if __name__ == '__main__': unittest.main()
