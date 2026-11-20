//+------------------------------------------------------------------+
//| Http.mqh — POST tới bridge TradeLens bằng WebRequest.            |
//+------------------------------------------------------------------+
#property strict

//--- POST body lên 127.0.0.1:port + path; trả mã HTTP (-1 khi WebRequest bị chặn).
int TradeLensPost(const int port, const string token, const string path, const string body, string &response)
  {
   string url = StringFormat("http://127.0.0.1:%d%s", port, path);
   string headers = "Content-Type: application/json\r\nX-TradeLens-Token: " + token + "\r\n";
   char data[], result[];
   string result_headers;
   StringToCharArray(body, data, 0, StringLen(body), CP_UTF8);
   int status = WebRequest("POST", url, headers, 5000, data, result, result_headers);
   if(status == -1)
      PrintFormat("TradeLens: WebRequest lỗi %d — thêm %s vào danh sách cho phép", GetLastError(), url);
   response = CharArrayToString(result, 0, WHOLE_ARRAY, CP_UTF8);
   return(status);
  }
//+------------------------------------------------------------------+

//--- POST một file nhị phân (ảnh PNG) kèm các header phụ; trả mã HTTP.
int TradeLensPostFile(const int port, const string token, const string path, const string extra_headers, const string file_name)
  {
   int handle = FileOpen(file_name, FILE_READ | FILE_BIN);
   if(handle == INVALID_HANDLE)
      return(-1);
   char data[];
   FileReadArray(handle, data);
   FileClose(handle);
   string url = StringFormat("http://127.0.0.1:%d%s", port, path);
   string headers = "Content-Type: image/png\r\nX-TradeLens-Token: " + token + "\r\n" + extra_headers;
   char result[];
   string result_headers;
   return(WebRequest("POST", url, headers, 10000, data, result, result_headers));
  }
//+------------------------------------------------------------------+
