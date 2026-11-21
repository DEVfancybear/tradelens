//+------------------------------------------------------------------+
//| Http.mqh — POST tới bridge TradeLens bằng WebRequest.            |
//+------------------------------------------------------------------+
#property strict

//--- Lỗi WebRequest gần nhất (GetLastError) khi hàm trả -1; 0 nếu không có lỗi.
int  g_tradelens_error = 0;
bool g_tradelens_alerted = false;

//--- Báo lỗi WebRequest bằng tiếng Việt; 4014 = URL chưa nằm trong danh sách cho phép của MT5.
void TradeLensReportError(const int error, const string url)
  {
   if(error == 4014)
     {
      string host = url;
      int slash = StringFind(host, "/", 8);        // bỏ đường dẫn, giữ http://127.0.0.1:cổng
      if(slash > 0)
         host = StringSubstr(host, 0, slash);
      string text = StringFormat("TradeLens: MT5 chặn WebRequest (lỗi 4014). Vào Tools → Options → Expert Advisors, "
                                 "bật 'Allow WebRequest for listed URL' và thêm %s", host);
      Print(text);
      if(!g_tradelens_alerted)
        {
         g_tradelens_alerted = true;
         Alert(text);
        }
      return;
     }
   if(error == 5200 || error == 5201 || error == 5203)
     {
      PrintFormat("TradeLens: không kết nối được app tại %s (lỗi %d) — app đã mở và cổng đúng chưa? Sẽ thử lại.", url, error);
      return;
     }
   if(error == 5202)
     {
      PrintFormat("TradeLens: app không trả lời kịp tại %s (quá thời gian chờ). Sẽ thử lại.", url);
      return;
     }
   PrintFormat("TradeLens: WebRequest lỗi %d khi gọi %s", error, url);
  }

//--- Chuỗi → mảng byte UTF-8, không kèm byte NUL cuối (số byte có thể lớn hơn số ký tự).
void TradeLensUtf8(const string text, char &data[])
  {
   int copied = StringToCharArray(text, data, 0, WHOLE_ARRAY, CP_UTF8);
   if(copied > 0)
      ArrayResize(data, copied - 1);
  }

//--- POST body lên 127.0.0.1:port + path; trả mã HTTP (-1 khi WebRequest lỗi, mã lỗi ở g_tradelens_error).
int TradeLensPost(const int port, const string token, const string path, const string body, string &response)
  {
   string url = StringFormat("http://127.0.0.1:%d%s", port, path);
   string headers = "Content-Type: application/json\r\nX-TradeLens-Token: " + token + "\r\n";
   char data[], result[];
   string result_headers;
   TradeLensUtf8(body, data);
   ResetLastError();
   int status = WebRequest("POST", url, headers, 5000, data, result, result_headers);
   if(status == -1)
     {
      g_tradelens_error = GetLastError();
      TradeLensReportError(g_tradelens_error, url);
     }
   else
      g_tradelens_error = 0;
   response = CharArrayToString(result, 0, WHOLE_ARRAY, CP_UTF8);
   return(status);
  }
//+------------------------------------------------------------------+

//--- POST một file nhị phân (ảnh PNG) kèm các header phụ; trả mã HTTP,
//--- -1 khi WebRequest lỗi, -2 khi không đọc được file.
int TradeLensPostFile(const int port, const string token, const string path, const string extra_headers, const string file_name)
  {
   int handle = FileOpen(file_name, FILE_READ | FILE_BIN);
   if(handle == INVALID_HANDLE)
      return(-2);
   char data[];
   int size = (int)FileSize(handle);
   ArrayResize(data, size);
   FileReadArray(handle, data, 0, size);
   FileClose(handle);
   string url = StringFormat("http://127.0.0.1:%d%s", port, path);
   string headers = "Content-Type: image/png\r\nX-TradeLens-Token: " + token + "\r\n" + extra_headers;
   char result[];
   string result_headers;
   ResetLastError();
   int status = WebRequest("POST", url, headers, 10000, data, result, result_headers);
   if(status == -1)
     {
      g_tradelens_error = GetLastError();
      TradeLensReportError(g_tradelens_error, url);
     }
   else
      g_tradelens_error = 0;
   return(status);
  }
//+------------------------------------------------------------------+
