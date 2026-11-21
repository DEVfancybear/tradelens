//+------------------------------------------------------------------+
//| TradeLensSync.mq5                                                |
//| Gửi deal của tài khoản sang app TradeLens qua HTTP 127.0.0.1.    |
//| Hợp đồng API: docs/sync-api.md                                   |
//+------------------------------------------------------------------+
#property copyright "TradeLens"
#property link      "https://github.com/DEVfancybear/tradelens"
#property version   "0.10"
#property strict

#include <TradeLens/Json.mqh>
#include <TradeLens/Http.mqh>

//--- Cài đặt (Tools → Options → Expert Advisors: Allow WebRequest cho http://127.0.0.1:7878)
input string InpToken        = "";    // Token (copy từ app: Nguồn dữ liệu)
input int    InpPort         = 7878;  // Cổng bridge
input int    InpBackfillDays = 90;    // Số ngày lịch sử gửi lần đầu
input int    InpTimerSeconds = 60;    // Chu kỳ gửi bù (giây)
input bool   InpScreenshots  = true;  // Tự chụp chart lúc vào và ra lệnh
input ENUM_TIMEFRAMES InpScreenshotTf = PERIOD_M15; // Khung giờ ưu tiên khi chụp
input int    InpScreenshotW  = 1600;  // Rộng ảnh (px)
input int    InpScreenshotH  = 900;   // Cao ảnh (px)

#define TL_BATCH_SIZE       200    // số deal tối đa mỗi request
#define TL_OVERLAP_SEC      3600   // đọc lùi so với con trỏ để bắt deal vào lịch sử muộn
#define TL_MAX_BACKOFF_SEC  60     // chờ tối đa giữa hai lần thử lại
#define TL_MAX_SHOTS        20     // ảnh chờ gửi tối đa
#define TL_MAX_SHOT_TRIES   5

//--- Con trỏ đồng bộ: time_msc của deal mới nhất app đã nhận. Lưu bằng GlobalVariable theo tài khoản
//--- nên còn nguyên sau khi tắt MT5 (MT5 xoá biến không dùng sau 4 tuần: lúc đó chỉ gửi lại, vô hại
//--- vì app khoá theo (tài khoản, ticket)).
long     g_login      = 0;
long     g_cursor_msc = 0;
ulong    g_sent_ticket[];   // deal đã gửi trong cửa sổ TL_OVERLAP_SEC, để không gửi lại
long     g_sent_msc[];

//--- Lịch gửi
bool     g_dirty      = true;   // có deal chưa chắc đã gửi (true để backfill ngay nhịp timer đầu)
datetime g_next_try   = 0;      // chưa thử lại trước thời điểm này (backoff)
datetime g_last_cycle = 0;      // lần gửi định kỳ thành công gần nhất
int      g_fail_count = 0;
int      g_last_status = 0;
ulong    g_watch      = 0;      // deal vừa phát sinh, chờ chắc chắn đã gửi
int      g_watch_tries = 0;

//--- Ảnh chờ gửi (giữ file đến khi app nhận)
string   g_shot_file[];
string   g_shot_headers[];
int      g_shot_tries[];

//+------------------------------------------------------------------+
int OnInit()
  {
   if(StringLen(InpToken) == 0)
     {
      Print("TradeLens: chưa nhập token");
      return(INIT_PARAMETERS_INCORRECT);
     }
   if(!EventSetTimer(1))   // tick 1 giây, việc thật chạy theo lịch trong OnTimer
     {
      Print("TradeLens: không đặt được timer");
      return(INIT_FAILED);
     }
   PrintFormat("TradeLens: sẵn sàng, gửi tới http://127.0.0.1:%d (lịch sử %d ngày lần đầu, gửi bù mỗi %d giây)",
               InpPort, InpBackfillDays, MathMax(InpTimerSeconds, 10));
   g_dirty = true;   // lịch sử InpBackfillDays ngày được gửi ở nhịp timer đầu tiên
   return(INIT_SUCCEEDED);
  }

//+------------------------------------------------------------------+
void OnDeinit(const int reason)
  {
   EventKillTimer();
   for(int i = 0; i < ArraySize(g_shot_file); i++)
      FileDelete(g_shot_file[i]);
  }

//+------------------------------------------------------------------+
//| Mỗi deal mới (vào, ra, nạp/rút) kích hoạt gửi ngay (mục tiêu ≤ 5 giây). |
//+------------------------------------------------------------------+
void OnTradeTransaction(const MqlTradeTransaction &trans,
                        const MqlTradeRequest &request,
                        const MqlTradeResult &result)
  {
   if(trans.type != TRADE_TRANSACTION_DEAL_ADD)
      return;

   // Đọc deal trước khi gửi: HistorySelect trong lúc gửi đổi vùng lịch sử đang chọn.
   bool is_trade = false;
   long entry = -1;
   long position = 0;
   string symbol = "";
   if(HistoryDealSelect(trans.deal))
     {
      long type = HistoryDealGetInteger(trans.deal, DEAL_TYPE);
      is_trade = (type == DEAL_TYPE_BUY || type == DEAL_TYPE_SELL);
      entry = HistoryDealGetInteger(trans.deal, DEAL_ENTRY);
      position = HistoryDealGetInteger(trans.deal, DEAL_POSITION_ID);
      symbol = HistoryDealGetString(trans.deal, DEAL_SYMBOL);
     }

   // Gửi deal trước, để app đã có lệnh (và hiện ô ghi kế hoạch) trước khi ảnh tới.
   g_dirty = true;
   g_watch = trans.deal;
   g_watch_tries = 0;
   if(TimeLocal() >= g_next_try)
      RunSync(false);

   if(InpScreenshots && is_trade)
     {
      if(entry == DEAL_ENTRY_IN)
         CaptureScreenshot(symbol, position, "entry");
      else if(!PositionSelectByTicket((ulong)position))
         CaptureScreenshot(symbol, position, "exit");   // đóng hẳn, không phải chốt một phần
     }
  }

//+------------------------------------------------------------------+
//| Chụp chart của symbol, xếp hàng và gửi sang app.                 |
//+------------------------------------------------------------------+
void CaptureScreenshot(const string symbol, const long position, const string moment)
  {
   long chart = FindChart(symbol);
   bool temporary = (chart == 0);
   if(temporary)
     {
      chart = ChartOpen(symbol, InpScreenshotTf);
      if(chart != 0)
         Sleep(500);   // chart mở tạm cần một chút để nạp nến
     }
   if(chart == 0)
     {
      PrintFormat("TradeLens: không mở được chart %s để chụp ảnh", symbol);
      return;
     }
   ChartRedraw(chart);
   string file = StringFormat("TradeLens\\%I64d-%s.png", position, moment);
   bool shot = ChartScreenShot(chart, file, InpScreenshotW, InpScreenshotH, ALIGN_RIGHT);
   if(temporary)
      ChartClose(chart);
   if(!shot)
     {
      PrintFormat("TradeLens: không chụp được ảnh chart %s (lỗi %d)", symbol, GetLastError());
      return;
     }
   string headers = StringFormat("X-TradeLens-Login: %I64d\r\nX-TradeLens-Server: %s\r\nX-TradeLens-Position: %I64d\r\nX-TradeLens-Moment: %s\r\n",
                                 AccountInfoInteger(ACCOUNT_LOGIN), AccountInfoString(ACCOUNT_SERVER), position, moment);
   QueueShot(file, headers);
   if(g_fail_count == 0)
      FlushShots();
  }

//+------------------------------------------------------------------+
//| Thêm ảnh vào hàng chờ (thay ảnh cùng tên nếu có).                |
//+------------------------------------------------------------------+
void QueueShot(const string file, const string headers)
  {
   int n = ArraySize(g_shot_file);
   for(int i = 0; i < n; i++)
      if(g_shot_file[i] == file)
        {
         g_shot_headers[i] = headers;
         g_shot_tries[i] = 0;
         return;
        }
   if(n >= TL_MAX_SHOTS)
     {
      FileDelete(g_shot_file[0]);   // bỏ ảnh cũ nhất
      DropShot(0);
      n--;
     }
   ArrayResize(g_shot_file, n + 1);
   ArrayResize(g_shot_headers, n + 1);
   ArrayResize(g_shot_tries, n + 1);
   g_shot_file[n] = file;
   g_shot_headers[n] = headers;
   g_shot_tries[n] = 0;
  }

//--- Bỏ phần tử i khỏi hàng chờ ảnh (không xoá file).
void DropShot(const int index)
  {
   int n = ArraySize(g_shot_file);
   for(int i = index; i < n - 1; i++)
     {
      g_shot_file[i] = g_shot_file[i + 1];
      g_shot_headers[i] = g_shot_headers[i + 1];
      g_shot_tries[i] = g_shot_tries[i + 1];
     }
   ArrayResize(g_shot_file, n - 1);
   ArrayResize(g_shot_headers, n - 1);
   ArrayResize(g_shot_tries, n - 1);
  }

//+------------------------------------------------------------------+
//| Gửi các ảnh đang chờ; ảnh gửi được hoặc bị từ chối thì xoá file.  |
//+------------------------------------------------------------------+
void FlushShots()
  {
   for(int i = ArraySize(g_shot_file) - 1; i >= 0; i--)
     {
      int status = TradeLensPostFile(InpPort, InpToken, "/api/v1/screenshot", g_shot_headers[i], g_shot_file[i]);
      bool done = (status >= 200 && status < 300);
      if(!done)
        {
         g_shot_tries[i]++;
         if(status == -2)
            done = true;   // file mất
         else if(status >= 400 && status < 500 && status != 401)
           {
            PrintFormat("TradeLens: app từ chối ảnh %s (mã %d)", g_shot_file[i], status);
            done = true;
           }
         else if(g_shot_tries[i] >= TL_MAX_SHOT_TRIES)
           {
            PrintFormat("TradeLens: bỏ ảnh %s sau %d lần gửi không được", g_shot_file[i], g_shot_tries[i]);
            done = true;
           }
        }
      if(done)
        {
         FileDelete(g_shot_file[i]);
         DropShot(i);
        }
     }
  }

//+------------------------------------------------------------------+
//| Chart đang mở symbol, ưu tiên khung giờ InpScreenshotTf; 0 nếu không có. |
//+------------------------------------------------------------------+
long FindChart(const string symbol)
  {
   long found = 0;
   for(long chart = ChartFirst(); chart >= 0; chart = ChartNext(chart))
     {
      if(ChartSymbol(chart) != symbol)
         continue;
      if(ChartPeriod(chart) == InpScreenshotTf)
         return(chart);
      if(found == 0)
         found = chart;
     }
   return(found);
  }

//+------------------------------------------------------------------+
//| Mỗi giây: gửi khi có deal chờ, hoặc định kỳ (gửi bù + cập nhật số dư). |
//+------------------------------------------------------------------+
void OnTimer()
  {
   datetime now = TimeLocal();
   if(now < g_next_try)
      return;
   bool periodic = (now - g_last_cycle >= MathMax(InpTimerSeconds, 10));
   if(!g_dirty && !periodic)
      return;
   RunSync(periodic);
  }

//+------------------------------------------------------------------+
//| Một lượt gửi kèm lịch thử lại: lỗi thì chờ 2, 4, 8... tối đa 60 giây. |
//+------------------------------------------------------------------+
void RunSync(const bool periodic)
  {
   bool ok = SyncDeals(periodic);
   datetime now = TimeLocal();
   if(ok)
     {
      g_fail_count = 0;
      g_dirty = false;
      g_next_try = 0;
      if(periodic)
         g_last_cycle = now;
      if(ArraySize(g_shot_file) > 0)
         FlushShots();
      if(g_watch != 0)
        {
         // Deal vừa phát sinh có thể chưa vào lịch sử khi gửi: thử lại sau 1 giây, tối đa 10 lần.
         if(WasSent(g_watch) || ++g_watch_tries > 10)
            g_watch = 0;
         else
           {
            g_dirty = true;
            g_next_try = now + 1;
           }
        }
      return;
     }
   g_fail_count++;
   g_dirty = true;
   int wait = 2;
   for(int i = 1; i < g_fail_count && wait < TL_MAX_BACKOFF_SEC; i++)
      wait *= 2;
   if(g_last_status == 401)
      wait = TL_MAX_BACKOFF_SEC;   // sai token: thử dồn dập cũng vô ích
   g_next_try = now + MathMin(wait, TL_MAX_BACKOFF_SEC);
  }

//+------------------------------------------------------------------+
//| Con trỏ đồng bộ của tài khoản đang đăng nhập.                    |
//+------------------------------------------------------------------+
string CursorKey(const long login)
  {
   return("TradeLens.cursor." + IntegerToString(login));
  }

void LoadCursor(const long login)
  {
   g_login = login;
   g_cursor_msc = 0;
   ArrayResize(g_sent_ticket, 0);
   ArrayResize(g_sent_msc, 0);
   string key = CursorKey(login);
   if(GlobalVariableCheck(key))
      g_cursor_msc = (long)GlobalVariableGet(key);   // double giữ chính xác số nguyên đến 2^53
  }

void SaveCursor()
  {
   GlobalVariableSet(CursorKey(g_login), (double)g_cursor_msc);
  }

bool WasSent(const ulong ticket)
  {
   for(int i = ArraySize(g_sent_ticket) - 1; i >= 0; i--)
      if(g_sent_ticket[i] == ticket)
         return(true);
   return(false);
  }

void MarkSent(const ulong ticket, const long msc)
  {
   int n = ArraySize(g_sent_ticket);
   ArrayResize(g_sent_ticket, n + 1, 512);
   ArrayResize(g_sent_msc, n + 1, 512);
   g_sent_ticket[n] = ticket;
   g_sent_msc[n] = msc;
  }

//--- Bỏ deal đã gửi nằm ngoài cửa sổ đọc lùi (chúng không còn được chọn nữa).
void PruneSent()
  {
   long limit = g_cursor_msc - (long)TL_OVERLAP_SEC * 1000;
   int keep = 0;
   int n = ArraySize(g_sent_ticket);
   for(int i = 0; i < n; i++)
      if(g_sent_msc[i] >= limit)
        {
         g_sent_ticket[keep] = g_sent_ticket[i];
         g_sent_msc[keep] = g_sent_msc[i];
         keep++;
        }
   ArrayResize(g_sent_ticket, keep);
   ArrayResize(g_sent_msc, keep);
  }

//--- Deal mới: sau con trỏ, hoặc trong cửa sổ đọc lùi mà chưa gửi.
bool IsNewDeal(const ulong ticket, const long msc)
  {
   if(msc > g_cursor_msc)
      return(true);
   if(msc < g_cursor_msc - (long)TL_OVERLAP_SEC * 1000)
      return(false);
   return(!WasSent(ticket));
  }

//+------------------------------------------------------------------+
//| Gửi mọi deal mới theo từng batch, con trỏ tiến sau mỗi batch app đã nhận. |
//| Trả true khi không còn gì chờ; false khi cần thử lại (con trỏ giữ nguyên |
//| ở batch lỗi, nên không mất deal).                                |
//+------------------------------------------------------------------+
bool SyncDeals(const bool periodic)
  {
   g_last_status = 0;
   long login = AccountInfoInteger(ACCOUNT_LOGIN);
   if(login <= 0)
      return(false);   // chưa đăng nhập
   if(login != g_login)
      LoadCursor(login);

   datetime now = TimeTradeServer();
   datetime from;
   if(g_cursor_msc > 0)
      from = (datetime)(g_cursor_msc / 1000 - TL_OVERLAP_SEC);
   else
      from = (datetime)((long)now - (long)MathMax(InpBackfillDays, 0) * 86400);
   if(!HistorySelect(from, now + 86400))
     {
      Print("TradeLens: không đọc được lịch sử giao dịch của tài khoản");
      return(false);
     }

   // 1. Gom deal mới (ticket + time_msc), xếp theo thời gian.
   long cand[][2];
   int n = 0;
   int total = HistoryDealsTotal();
   for(int i = 0; i < total; i++)
     {
      ulong ticket = HistoryDealGetTicket(i);
      if(ticket == 0)
         continue;
      long msc = HistoryDealGetInteger(ticket, DEAL_TIME_MSC);
      if(!IsNewDeal(ticket, msc))
         continue;
      ArrayResize(cand, n + 1, 256);
      cand[n][0] = msc;
      cand[n][1] = (long)ticket;
      n++;
     }
   if(n > 1)
      ArraySort(cand);   // theo cột đầu (time_msc), cả hàng đi cùng

   string account = TradeLensAccountJson(ServerOffsetMinutes());
   string response;

   // Không có deal mới: chỉ cập nhật số dư/equity (nhịp định kỳ), cũng là cách báo "EA còn sống".
   if(n == 0)
     {
      if(!periodic)
         return(true);
      int status = TradeLensPost(InpPort, InpToken, "/api/v1/account", "{\"account\":" + account + "}", response);
      return(StatusOk(status, response));
     }

   // 2. Gửi từng batch; mỗi batch xong thì ghi nhận và dời con trỏ.
   bool all_sent = true;
   for(int start = 0; start < n && all_sent; start += TL_BATCH_SIZE)
     {
      int count = MathMin(TL_BATCH_SIZE, n - start);
      string deals[];
      ulong tickets[];
      ArrayResize(deals, count);
      ArrayResize(tickets, count);
      int built = 0;
      for(int k = 0; k < count; k++)
        {
         ulong ticket = (ulong)cand[start + k][1];
         string json = BuildDealJson(ticket);
         if(StringLen(json) == 0)
            continue;   // deal biến mất khỏi lịch sử: bỏ qua
         deals[built] = json;
         tickets[built] = ticket;
         built++;
        }
      if(built > 0)
        {
         string body = TradeLensBatchJson(account, deals, 0, built);
         int status = TradeLensPost(InpPort, InpToken, "/api/v1/deals", body, response);
         if(status == 400)
            all_sent = SendOneByOne(account, deals, tickets, built);   // tách ra để một deal hỏng không chặn cả lịch sử
         else
            all_sent = StatusOk(status, response);
        }
      if(all_sent)
        {
         for(int k = 0; k < count; k++)
           {
            long msc = cand[start + k][0];
            MarkSent((ulong)cand[start + k][1], msc);
            if(msc > g_cursor_msc)
               g_cursor_msc = msc;
           }
         SaveCursor();
        }
     }
   PruneSent();
   return(all_sent);
  }

//--- Gửi từng deal một sau khi cả batch bị 400. Deal bị app từ chối bị bỏ qua (có ghi log);
//--- lỗi khác thì dừng và thử lại cả batch sau (app khoá theo ticket nên gửi lại vô hại).
bool SendOneByOne(const string account, const string &deals[], const ulong &tickets[], const int count)
  {
   string response;
   for(int i = 0; i < count; i++)
     {
      string one[1];
      one[0] = deals[i];
      string body = TradeLensBatchJson(account, one, 0, 1);
      int status = TradeLensPost(InpPort, InpToken, "/api/v1/deals", body, response);
      if(status == 400)
        {
         PrintFormat("TradeLens: app từ chối deal %I64u (JSON sai), bỏ qua: %s", tickets[i], response);
         continue;
        }
      if(!StatusOk(status, response))
         return(false);
     }
   return(true);
  }

//--- 2xx là xong; còn lại in thông báo tiếng Việt và trả false.
bool StatusOk(const int status, const string &response)
  {
   g_last_status = status;
   if(status >= 200 && status < 300)
      return(true);
   if(status == -1)
      return(false);   // TradeLensPost đã báo (4014, không kết nối...)
   if(status == 401)
      Print("TradeLens: app từ chối token (401) — token trong ô InpToken của EA phải trùng token ở màn Nguồn dữ liệu của app");
   else
      PrintFormat("TradeLens: app trả lỗi %d: %s", status, response);
   return(false);
  }

//+------------------------------------------------------------------+
//| JSON của một deal kèm SL/TP và tiền rủi ro (deal vào lệnh).      |
//+------------------------------------------------------------------+
string BuildDealJson(const ulong ticket)
  {
   if(!HistoryDealSelect(ticket))
      return("");
   long type = HistoryDealGetInteger(ticket, DEAL_TYPE);
   long entry = HistoryDealGetInteger(ticket, DEAL_ENTRY);
   string symbol = HistoryDealGetString(ticket, DEAL_SYMBOL);
   double price = HistoryDealGetDouble(ticket, DEAL_PRICE);
   double volume = HistoryDealGetDouble(ticket, DEAL_VOLUME);
   double sl = 0.0;
   double tp = 0.0;
   double risk = 0.0;
   if(type == DEAL_TYPE_BUY || type == DEAL_TYPE_SELL)
     {
      DealLevels(ticket, sl, tp);
      if(entry == DEAL_ENTRY_IN)
         risk = RiskMoney(symbol, price, sl, volume);
     }
   return(TradeLensDealJson(ticket, sl, tp, risk));
  }

//+------------------------------------------------------------------+
//| SL/TP của deal. Thứ tự: lệnh sinh ra deal; position còn mở (mức hiện tại); |
//| với deal ra, lệnh đã mở position (position id = ticket lệnh mở).  |
//+------------------------------------------------------------------+
void DealLevels(const ulong ticket, double &sl, double &tp)
  {
   sl = 0.0;
   tp = 0.0;
   if(!HistoryDealSelect(ticket))
      return;
   ulong order = (ulong)HistoryDealGetInteger(ticket, DEAL_ORDER);
   ulong position = (ulong)HistoryDealGetInteger(ticket, DEAL_POSITION_ID);
   long entry = HistoryDealGetInteger(ticket, DEAL_ENTRY);

   OrderLevels(order, sl, tp);
   if((sl <= 0.0 || tp <= 0.0) && position != 0 && PositionSelectByTicket(position))
     {
      if(sl <= 0.0)
         sl = PositionGetDouble(POSITION_SL);
      if(tp <= 0.0)
         tp = PositionGetDouble(POSITION_TP);
     }
   if(entry != DEAL_ENTRY_IN && position != 0 && (sl <= 0.0 || tp <= 0.0))
     {
      double open_sl = 0.0;
      double open_tp = 0.0;
      OrderLevels(position, open_sl, open_tp);
      if(sl <= 0.0)
         sl = open_sl;
      if(tp <= 0.0)
         tp = open_tp;
     }
  }

//--- SL/TP ghi trên một lệnh trong lịch sử (0 nếu không đọc được).
void OrderLevels(const ulong order, double &sl, double &tp)
  {
   sl = 0.0;
   tp = 0.0;
   if(order == 0 || !HistoryOrderSelect(order))
      return;
   sl = HistoryOrderGetDouble(order, ORDER_SL);
   tp = HistoryOrderGetDouble(order, ORDER_TP);
  }

//+------------------------------------------------------------------+
//| Độ lệch giờ server so với UTC, phút (gửi kèm account).           |
//| Làm tròn về bội số 15 phút (múi giờ thật), bỏ sai số vài giây giữa |
//| đồng hồ máy và server.                                           |
//+------------------------------------------------------------------+
int ServerOffsetMinutes()
  {
   double seconds = (double)(TimeTradeServer() - TimeGMT());
   return((int)(MathRound(seconds / 900.0) * 15.0));
  }

//+------------------------------------------------------------------+
//| Tiền rủi ro tại SL của một deal vào lệnh.                        |
//+------------------------------------------------------------------+
double RiskMoney(const string symbol, const double price, const double sl, const double volume)
  {
   if(sl <= 0.0)
      return(0.0);
   double tick_size  = SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_SIZE);
   double tick_value = SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_VALUE_LOSS);
   if(tick_size <= 0.0)
      return(0.0);
   return(MathAbs(price - sl) / tick_size * tick_value * volume);
  }
//+------------------------------------------------------------------+
