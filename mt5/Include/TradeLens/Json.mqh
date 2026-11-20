//+------------------------------------------------------------------+
//| Json.mqh — dựng JSON DealBatch (docs/sync-api.md) không cần DLL. |
//+------------------------------------------------------------------+
#property strict

//--- Thoát chuỗi theo JSON.
string TradeLensEscape(const string s)
  {
   string out = s;
   StringReplace(out, "\\", "\\\\");
   StringReplace(out, "\"", "\\\"");
   StringReplace(out, "\n", "\\n");
   StringReplace(out, "\r", "\\r");
   return(out);
  }

//--- Phần "account" của batch.
string TradeLensAccountJson(const int server_offset_min)
  {
   // TODO(v0.1): login, server, broker, name, currency, balance, equity, server_offset_min
   return("{}");
  }

//--- Một deal của lịch sử (đã HistorySelect).
string TradeLensDealJson(const ulong ticket, const double risk)
  {
   // TODO(v0.1): HistoryDealGet* → ticket, order, position_id, time_msc, symbol,
   // type, entry, volume, price, commission, swap, fee, profit, magic, comment, sl, tp, risk
   return("{}");
  }
//+------------------------------------------------------------------+
